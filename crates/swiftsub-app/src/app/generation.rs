use super::*;
use swiftsub_core::parser::ParseOutput;

fn generate_and_save(
    store: &Mutex<Store>,
    profile_id: i64,
    nodes: &[Node],
    options: &GenerateOptions,
) -> Result<swiftsub_core::GeneratedProfile, String> {
    let profile = generate_profile(nodes, options).map_err(|error| error.to_string())?;
    store
        .lock()
        .map_err(|error| error.to_string())?
        .save_profile_yaml(profile_id, &profile.yaml)
        .map_err(|error| error.to_string())?;
    Ok(profile)
}

type CachedParse = (String, Result<ParseOutput, String>);

type CachedGeneration = (String, Result<swiftsub_core::GeneratedProfile, String>);

#[derive(Default)]
pub(super) struct GenerationCache(std::cell::RefCell<VecDeque<CachedGeneration>>);

impl GenerationCache {
    pub(super) fn generate(
        &self,
        nodes: &[Node],
        options: &GenerateOptions,
    ) -> Result<swiftsub_core::GeneratedProfile, String> {
        let mut options = options.clone();
        if let Some(modules) = &mut options.enabled_modules {
            modules.sort();
        }
        options.disabled_rule_sets.sort();
        let node_values: Vec<_> = nodes
            .iter()
            .map(|node| {
                (
                    node.proxy.fields(),
                    &node.source_key,
                    &node.source_id,
                    &node.source_name,
                    node.region.label(),
                    node.enabled,
                    node.listen_port,
                )
            })
            .collect();
        let key = serde_json::to_string(&(&options, node_values)).map_err(|e| e.to_string())?;
        if let Some((_, result)) = self
            .0
            .borrow()
            .iter()
            .find(|(previous, _)| previous == &key)
        {
            return result.clone();
        }
        let result = generate_profile(nodes, &options).map_err(|e| e.to_string());
        let mut cache = self.0.borrow_mut();
        if cache.len() >= 4 {
            cache.pop_front();
        }
        cache.push_back((key, result.clone()));
        result
    }
}

#[derive(Clone, Default)]
pub(super) struct ParseCache(Arc<Mutex<HashMap<i64, CachedParse>>>);

impl ParseCache {
    pub(super) fn parse(&self, id: i64, payload: &str) -> Result<ParseOutput, String> {
        if let Ok(cache) = self.0.lock()
            && let Some((previous, parsed)) = cache.get(&id)
            && previous == payload
        {
            return parsed.clone();
        }
        let parsed = parse_content(payload).map_err(|error| error.to_string());
        if let Ok(mut cache) = self.0.lock() {
            cache.insert(id, (payload.to_owned(), parsed.clone()));
        }
        parsed
    }

    pub(super) fn retain(&self, subscriptions: &[StoredSubscription]) {
        if let Ok(mut cache) = self.0.lock() {
            cache.retain(|id, _| subscriptions.iter().any(|sub| sub.id == *id));
        }
    }
}

impl SwiftSubApp {
    pub(crate) fn projected_profile(
        &self,
        cx: &Context<Self>,
    ) -> Result<swiftsub_core::GeneratedProfile, String> {
        self.workspace_projected_options(cx)
            .and_then(|options| self.generation_cache.generate(&self.nodes, &options))
    }
    pub(super) fn rebuild_nodes(&mut self, cx: &mut Context<Self>) {
        let mut reports = Vec::new();
        let mut all_nodes = Vec::new();
        let mut seen = HashSet::new();
        let mut warnings = Vec::new();
        let subscriptions = self.subscriptions.clone();

        let overrides_by_source = {
            let store = lock_store(&self.store);
            let mut overrides = HashMap::new();
            for subscription in &subscriptions {
                match store.list_node_overrides(subscription.id) {
                    Ok(values) => {
                        overrides.insert(subscription.id, values);
                    }
                    Err(error) => {
                        self.store_error = Some(format!("节点编辑状态读取失败：{error}"));
                        cx.notify();
                        return;
                    }
                }
            }
            overrides
        };
        {
            for subscription in subscriptions.iter().filter(|subscription| {
                subscription.enabled && self.profile_includes(subscription.id)
            }) {
                let Some(payload) = subscription.payload.as_deref() else {
                    continue;
                };
                match self.parsed_sources.parse(subscription.id, payload) {
                    Ok(parsed) => {
                        reports.extend(
                            parsed
                                .report
                                .iter()
                                .cloned()
                                .map(|item| (subscription.name.clone(), item)),
                        );
                        for warning in parsed.warnings {
                            warnings.push(format!("{} · {}", subscription.name, warning.message));
                        }
                        let mut nodes = build_nodes(
                            parsed.proxies,
                            subscription.id.to_string(),
                            subscription.name.clone(),
                        );
                        let overrides = &overrides_by_source[&subscription.id];
                        for node in &mut nodes {
                            let source_key = node.source_key.clone();
                            if self.deleted_nodes.contains(&source_key) {
                                continue;
                            }
                            if let Some(value) = overrides.get(&source_key) {
                                if let Some(extras) = &value.extras
                                    && let Ok(edited) = Proxy::from_yaml_text(extras)
                                {
                                    node.proxy = edited;
                                }
                                if let Some(alias) = &value.alias {
                                    node.set_name(alias.clone());
                                }
                                if let Some(enabled) = value.enabled {
                                    node.enabled = enabled;
                                }
                                node.listen_port = value.listen_port;
                            }
                            if self.disabled_nodes.contains(&source_key) {
                                node.enabled = false;
                            }
                            node.region = detect_region(node.name());
                            if seen.insert(node.proxy.identity_key()) {
                                all_nodes.push(node.clone());
                            }
                        }
                    }
                    Err(error) => warnings.push(format!("{}：{error}", subscription.name)),
                }
            }
        }

        if !self.node_order.is_empty() {
            let positions: HashMap<&str, usize> = self
                .node_order
                .iter()
                .enumerate()
                .map(|(index, key)| (key.as_str(), index))
                .collect();
            all_nodes.sort_by_key(|node| {
                positions
                    .get(node.source_key.as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
            });
        }

        self.nodes = all_nodes;
        self.apply_workspace_node_order();
        self.source_import_report = reports;
        self.parse_warnings = warnings;
        self.regenerate(cx);
    }

    pub fn regenerate(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.save_active_profile() {
            self.store_error = Some(format!("配置保存失败：{error}"));
            cx.notify();
            return;
        }
        if self.nodes.is_empty() {
            self.gen_warnings = self.parse_warnings.clone();
            self.gen_warnings
                .push("没有可用节点，继续提供上次成功配置".into());
            self.restore_last_output();
            self.publish_profiles();
            cx.notify();
            return;
        }

        let mut options = self.options.clone();
        if let Some(defaults) = &self.workspace.global_defaults {
            defaults.apply(&mut options);
        }
        options.enabled_modules = Some(self.enabled_modules.iter().cloned().collect());
        options.disabled_rule_sets = self.disabled_rule_sets.iter().cloned().collect();
        options.custom_rule_sets = self.normalize_rule_set_bindings(&self.custom_rule_sets);
        options.custom_groups = self
            .template_baseline
            .as_ref()
            .map(|b| b.custom_groups.clone())
            .unwrap_or_else(|| self.custom_groups.clone());
        options.chains = self
            .template_baseline
            .as_ref()
            .map(|b| b.chains.clone())
            .unwrap_or_else(|| self.chains.clone());

        let preview_was_empty = self.preview.is_empty();
        let generated = generate_and_save(&self.store, self.profile_id, &self.nodes, &options);
        match generated {
            Ok(profile) => {
                self.generation_report =
                    serde_json::to_string_pretty(&profile.trace).unwrap_or_default();
                self.preview = profile.yaml;
                if preview_was_empty {
                    self.preview_scroll.set_offset(point(px(0.0), px(0.0)));
                }
                self.stats = profile.stats;
                self.gen_warnings = profile.warnings;

                self.publish_profiles();
            }
            Err(error) => {
                self.restore_last_output();
                self.gen_warnings = vec![format!("生成或保存失败，继续提供上次成功配置：{error}")];
                self.publish_profiles();
            }
        }
        cx.notify();
    }

    fn restore_last_output(&mut self) {
        let result = lock_store(&self.store).get_profile_by_token(&self.profile_token);
        match result {
            Ok(Some(profile)) => self.preview = profile.last_yaml.unwrap_or_default(),
            Ok(None) => self.preview.clear(),
            Err(error) => self.store_error = Some(format!("上次成功配置读取失败：{error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes() -> Vec<Node> {
        let mut proxy = Proxy::new("trojan", "test", "example.org", 443);
        proxy.insert("password", "test-only");
        build_nodes(vec![proxy], "1", "test")
    }

    #[test]
    fn invalid_generation_keeps_last_successful_database_output() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let profile = store.lock().unwrap().get_or_create_profile("test").unwrap();
        let valid =
            generate_and_save(&store, profile.id, &nodes(), &GenerateOptions::default()).unwrap();
        let invalid = GenerateOptions {
            mixed_port: 0,
            ..Default::default()
        };
        assert!(generate_and_save(&store, profile.id, &nodes(), &invalid).is_err());
        assert!(generate_and_save(&store, profile.id, &[], &GenerateOptions::default()).is_err());
        assert_eq!(
            store
                .lock()
                .unwrap()
                .get_profile_by_token(&profile.token)
                .unwrap()
                .unwrap()
                .last_yaml
                .as_deref(),
            Some(valid.yaml.as_str())
        );
        assert!(generate_and_save(&store, -1, &nodes(), &GenerateOptions::default()).is_err());
    }

    #[test]
    fn changed_payload_invalidates_parse_cache() {
        let cache = ParseCache::default();
        let first = "trojan://password@first.example:443#first";
        let second = "trojan://password@second.example:443#second";
        assert_eq!(cache.parse(1, first).unwrap().proxies[0].name(), "first");
        assert_eq!(cache.parse(1, second).unwrap().proxies[0].name(), "second");
        assert_eq!(cache.parse(2, first).unwrap().proxies[0].name(), "first");
        cache.retain(&[]);
        assert!(cache.0.lock().unwrap().is_empty());
    }

    #[test]
    fn hundred_node_cache_tracks_node_and_routing_changes_and_measures_reuse() {
        let links = (0..100)
            .map(|i| format!("trojan://test-only@node-{i}.example.org:443#HK-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut nodes = build_nodes(parse_content(&links).unwrap().proxies, "test", "test");
        let mut options = GenerateOptions {
            preset: Preset::Minimal,
            ..Default::default()
        };
        let cache = GenerationCache::default();
        let first = cache.generate(&nodes, &options).unwrap();
        let original_key = cache.0.borrow().back().unwrap().0.clone();
        let mut cached = Vec::new();
        let mut uncached = Vec::new();
        for _ in 0..20 {
            let start = std::time::Instant::now();
            let result = cache.generate(&nodes, &options).unwrap();
            cached.push(start.elapsed().as_secs_f64() * 1000.);
            assert_eq!(result.yaml, first.yaml);
            let start = std::time::Instant::now();
            let expected = generate_profile(&nodes, &options).unwrap();
            uncached.push(start.elapsed().as_secs_f64() * 1000.);
            assert_eq!(expected.yaml, first.yaml);
        }
        assert_eq!(cache.0.borrow().back().unwrap().0, original_key);
        cached.sort_by(f64::total_cmp);
        uncached.sort_by(f64::total_cmp);
        println!(
            "nodes=100 debug_generation_median_ms={:.3} debug_cache_median_ms={:.3}",
            uncached[10], cached[10]
        );
        nodes[0].set_name("renamed");
        assert_eq!(
            cache.generate(&nodes, &options).unwrap().yaml,
            generate_profile(&nodes, &options).unwrap().yaml
        );
        assert_ne!(cache.0.borrow().back().unwrap().0, original_key);
        nodes[1].enabled = false;
        assert_eq!(cache.generate(&nodes, &options).unwrap().stats.proxies, 99);
        options.dns_override = true;
        options.dns.domestic = vec!["https://dns.example.org/dns-query".into()];
        assert_eq!(
            cache.generate(&nodes, &options).unwrap().yaml,
            generate_profile(&nodes, &options).unwrap().yaml
        );
        options.extra_yaml = "x-cache-test: true".into();
        assert!(
            cache
                .generate(&nodes, &options)
                .unwrap()
                .yaml
                .contains("x-cache-test")
        );
        options.mixed_port = 0;
        assert!(cache.generate(&nodes, &options).is_err());
        options.mixed_port = 7890;
        assert!(cache.generate(&nodes, &options).is_ok());
    }
}
