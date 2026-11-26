use super::*;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};

pub(super) struct PublishJob {
    pub store: Arc<Mutex<Store>>,
    pub publisher: swiftsub_server::ProfilePublisher,
    pub active_profile: i64,
    pub subscriptions: Vec<StoredSubscription>,
    pub node_order: Vec<String>,
    pub parsed_sources: generation::ParseCache,
    pub snapshots: HashMap<String, ProfileSnapshot>,
}

#[derive(Default)]
pub(super) struct PublicationWorker {
    sender: OnceLock<Sender<(u64, PublishJob)>>,
    revision: Arc<Mutex<u64>>,
    error: Arc<Mutex<Option<String>>>,
}

impl PublicationWorker {
    pub(super) fn submit(&self, job: PublishJob) {
        let mut revision = self
            .revision
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *revision += 1;
        // Existing persisted bodies are immediately available, including newly saved records.
        job.publisher.replace_profiles(job.snapshots.clone());
        let shared_revision = Arc::clone(&self.revision);
        let shared_error = Arc::clone(&self.error);
        let sender = self.sender.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<(u64, PublishJob)>();
            if let Err(error) = std::thread::Builder::new()
                .name("swiftsub-publish".into())
                .spawn(move || {
                    while let Ok(mut pending) = receiver.recv() {
                        while let Ok(newer) = receiver.try_recv() {
                            pending = newer;
                        }
                        let (revision, job) = pending;
                        if let Err(error) = rebuild(revision, job, &shared_revision) {
                            *shared_error
                                .lock()
                                .unwrap_or_else(|poison| poison.into_inner()) = Some(error);
                        }
                    }
                })
            {
                *self
                    .error
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) =
                    Some(format!("后台发布线程启动失败：{error}"));
            }
            sender
        });
        let _ = sender.send((*revision, job));
    }

    pub(super) fn take_error(&self) -> Option<String> {
        self.error.lock().ok()?.take()
    }
}

fn rebuild(revision: u64, job: PublishJob, shared_revision: &Mutex<u64>) -> Result<(), String> {
    let profiles = {
        let store = job.store.lock().map_err(|error| error.to_string())?;
        store.list_profiles().map_err(|error| error.to_string())?
    };
    let mut published = job.snapshots.clone();
    let mut outputs = Vec::new();
    for profile in profiles
        .into_iter()
        .filter(|profile| profile.id != job.active_profile)
    {
        if *shared_revision
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            != revision
        {
            return Ok(());
        }
        let config = profile
            .options_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<profiles::ProfileConfig>(json).ok());
        let userinfo = config.as_ref().and_then(|config| {
            profiles::profile_userinfo(&job.subscriptions, config.membership.as_ref())
        });
        let generated = config
            .and_then(|config| {
                let mut options = config
                    .routing
                    .generate_options(&GenerateOptions::default())
                    .ok()?;
                if let Some(defaults) = workspace::GenerationDefaults::from_store(&job.store) {
                    defaults.apply(&mut options);
                }
                let overrides_by_source = {
                    let store = job.store.lock().ok()?;
                    let mut values = HashMap::new();
                    for sub in &job.subscriptions {
                        values.insert(sub.id, store.list_node_overrides(sub.id).ok()?);
                    }
                    values
                };
                let deleted_nodes: HashSet<String> = job
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| store.get_setting("deleted_nodes").ok().flatten())
                    .and_then(|value| serde_json::from_str(&value).ok())
                    .unwrap_or_default();
                let mut nodes = Vec::new();
                for sub in job.subscriptions.iter().filter(|s| {
                    s.enabled
                        && config
                            .membership
                            .as_ref()
                            .is_none_or(|ids| ids.contains(&s.id))
                }) {
                    if let Some(parsed) = sub
                        .payload
                        .as_deref()
                        .and_then(|p| job.parsed_sources.parse(sub.id, p).ok())
                    {
                        let mut imported =
                            build_nodes(parsed.proxies, sub.id.to_string(), sub.name.clone());
                        imported.retain(|n| !deleted_nodes.contains(&n.source_key));
                        let overrides = &overrides_by_source[&sub.id];
                        for n in &mut imported {
                            if let Some(o) = overrides.get(&n.source_key) {
                                if let Some(extras) = &o.extras
                                    && let Ok(proxy) = Proxy::from_yaml_text(extras)
                                {
                                    n.proxy = proxy;
                                }
                                if let Some(alias) = &o.alias {
                                    n.set_name(alias.clone());
                                }
                                if let Some(enabled) = o.enabled {
                                    n.enabled = enabled;
                                }
                                n.listen_port = o.listen_port;
                            }
                        }
                        for n in &mut imported {
                            n.region = detect_region(n.name());
                        }
                        nodes.extend(imported);
                    }
                }
                let mut seen = HashSet::new();
                nodes.retain(|n| seen.insert(n.proxy.identity_key()));
                let node_order = config.node_order.as_ref().unwrap_or(&job.node_order);
                if !node_order.is_empty() {
                    let positions: HashMap<_, _> =
                        node_order.iter().enumerate().map(|(i, k)| (k, i)).collect();
                    nodes.sort_by_key(|n| {
                        positions.get(&n.source_key).copied().unwrap_or(usize::MAX)
                    });
                }
                generate_profile(&nodes, &options).ok().map(|p| p.yaml)
            })
            .or_else(|| profile.last_yaml.clone())
            .unwrap_or_default();
        outputs.push((profile.id, profile.options_json, generated.clone()));
        published.insert(
            profile.token,
            ProfileSnapshot {
                yaml: generated,
                userinfo_header: userinfo,
                update_interval_hours: 24,
            },
        );
    }
    // Serialize acceptance with new submissions so an old task cannot replace newer output.
    let current = shared_revision
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if *current != revision {
        return Ok(());
    }
    let store = job.store.lock().map_err(|error| error.to_string())?;
    if !store
        .save_generated_outputs(&outputs)
        .map_err(|error| error.to_string())?
    {
        return Ok(());
    }
    job.publisher.replace_profiles(published);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn body(server: &LocalServer, token: &str) -> String {
        let mut stream = std::net::TcpStream::connect(server.local_addr()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            stream,
            "GET /sub/{token}/test.yaml HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        response.split_once("\r\n\r\n").unwrap().1.to_owned()
    }

    #[test]
    fn hundred_node_background_refresh_preserves_edits_snapshots_and_rejects_stale_jobs() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let (profile, subscriptions) = {
            let store = store.lock().unwrap();
            let id = store
                .add_subscription(&NewSubscription::url("test", "https://example.org/sub"))
                .unwrap();
            let payload = (0..100)
                .map(|i| format!("trojan://password@node-{i}.example.org:443#Node-{i}"))
                .collect::<Vec<_>>()
                .join("\n");
            store
                .save_fetch_success(id, &payload, None, None, &TrafficSnapshot::default(), 100)
                .unwrap();
            let nodes = build_nodes(
                parse_content(&payload).unwrap().proxies,
                id.to_string(),
                "test",
            );
            store
                .set_node_override_alias(id, &nodes[0].source_key, "My alias")
                .unwrap();
            store
                .set_node_override_enabled(id, &nodes[1].source_key, false)
                .unwrap();
            let profile = store.get_or_create_profile("inactive").unwrap();
            let recipe = serde_json::json!({"routing": {
                "preset": "minimal", "enabled_modules": ["select", "auto", "final"],
                "test_url": "https://example.org/204", "test_interval": 300,
                "mixed_port": 7897, "allow_lan": false,
                "group_order": ["漏网之鱼", "自动选择", "节点选择"]
            }, "node_order": [nodes[99].source_key, nodes[0].source_key]});
            store
                .save_profile_options(profile.id, &recipe.to_string())
                .unwrap();
            store
                .save_profile_yaml(profile.id, "previous-output")
                .unwrap();
            (profile, store.list_subscriptions().unwrap())
        };
        let server = LocalServer::start(
            ServerConfig {
                port: 0,
                token: "active".into(),
                ..Default::default()
            },
            ProfileSnapshot {
                yaml: "active-output".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let job = || PublishJob {
            store: Arc::clone(&store),
            publisher: server.publisher(),
            active_profile: -1,
            subscriptions: subscriptions.clone(),
            node_order: Vec::new(),
            parsed_sources: generation::ParseCache::default(),
            snapshots: HashMap::from([(
                "saved".into(),
                ProfileSnapshot {
                    yaml: "saved-output".into(),
                    ..Default::default()
                },
            )]),
        };
        rebuild(1, job(), &Mutex::new(2)).unwrap();
        assert_eq!(
            store
                .lock()
                .unwrap()
                .get_profile_by_token(&profile.token)
                .unwrap()
                .unwrap()
                .last_yaml
                .as_deref(),
            Some("previous-output")
        );
        rebuild(2, job(), &Mutex::new(2)).unwrap();
        let generated = body(&server, &profile.token);
        let parsed = parse_content(&generated).unwrap();
        assert_eq!(parsed.proxies.len(), 99);
        assert_eq!(parsed.proxies[0].name(), "Node-99");
        assert_eq!(parsed.proxies[1].name(), "My alias");
        let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(&generated).unwrap();
        assert_eq!(yaml["proxy-groups"][0]["name"].as_str(), Some("漏网之鱼"));
        assert!(
            parsed
                .proxies
                .iter()
                .any(|proxy| proxy.name() == "My alias")
        );
        assert!(!parsed.proxies.iter().any(|proxy| proxy.name() == "Node-1"));
        assert_eq!(body(&server, "saved"), "saved-output");
        // Deleted nodes recorded in settings must also be excluded from background published profiles.
        let deleted_key = parse_content(
            store.lock().unwrap().list_subscriptions().unwrap()[0]
                .payload
                .as_deref()
                .unwrap(),
        )
        .unwrap()
        .proxies[2]
            .identity_key();
        store
            .lock()
            .unwrap()
            .set_setting(
                "deleted_nodes",
                &serde_json::to_string(&HashSet::from([deleted_key])).unwrap(),
            )
            .unwrap();
        rebuild(3, job(), &Mutex::new(3)).unwrap();
        let after_delete = parse_content(&body(&server, &profile.token)).unwrap();
        assert_eq!(after_delete.proxies.len(), 98);
        assert!(!after_delete.proxies.iter().any(|p| p.name() == "Node-2"));
        let mut invalid = job();
        invalid.subscriptions.clear();
        rebuild(4, invalid, &Mutex::new(4)).unwrap();
        assert_eq!(
            parse_content(&body(&server, &profile.token))
                .unwrap()
                .proxies
                .len(),
            98
        );
        assert_eq!(body(&server, "saved"), "saved-output");
    }
}
