use super::*;

pub(super) struct FetchJob {
    pub(super) id: i64,
    url: String,
    name: String,
    cache: CacheValidators,
    options: FetchOptions,
    generation: u64,
}

fn next_job(queue: &mut VecDeque<FetchJob>, active: &HashSet<i64>) -> Option<FetchJob> {
    if active.len() >= 2 {
        return None;
    }
    let index = queue.iter().position(|job| !active.contains(&job.id))?;
    queue.remove(index)
}

fn accepts_result(
    generation: u64,
    url: &str,
    current_version: Option<&u64>,
    subscription: Option<&StoredSubscription>,
) -> bool {
    current_version == Some(&generation)
        && subscription.is_some_and(|sub| sub.enabled && sub.url.as_deref() == Some(url))
}

fn fetch_validated_subscription(
    id: i64,
    url: &str,
    options: &FetchOptions,
    validators: &CacheValidators,
    cached_payload: Option<&str>,
    parsed_sources: &generation::ParseCache,
) -> (Result<FetchOutcome, String>, usize) {
    let cached = cached_payload
        .and_then(|payload| parsed_sources.parse(id, payload).ok())
        .filter(|parsed| {
            parsed
                .proxies
                .iter()
                .any(|proxy| proxy.fatal_issue().is_none())
        });
    let empty_cache = CacheValidators::default();
    let validators = if cached.is_some() {
        validators
    } else {
        &empty_cache
    };
    let mut outcome =
        fetch_subscription(url, options, validators).map_err(|error| error.to_string());
    if matches!(outcome, Ok(FetchOutcome::NotModified { .. })) && cached.is_none() {
        outcome = fetch_subscription(url, options, &empty_cache).map_err(|error| error.to_string());
    }
    match outcome {
        Ok(FetchOutcome::Fetched(result)) => match parsed_sources.parse(id, &result.content) {
            Ok(parsed)
                if parsed
                    .proxies
                    .iter()
                    .any(|proxy| proxy.fatal_issue().is_none()) =>
            {
                let count = parsed.proxies.len();
                (Ok(FetchOutcome::Fetched(result)), count)
            }
            Ok(_) => (Err("订阅没有可用节点，保留上次成功内容".into()), 0),
            Err(error) => (Err(format!("订阅解析失败，保留上次成功内容：{error}")), 0),
        },
        Ok(FetchOutcome::NotModified { .. }) if cached.is_none() => (
            Err("订阅服务器返回 304，但本地没有可用缓存；请检查订阅地址后重试".into()),
            0,
        ),
        other => (other, cached.map_or(0, |parsed| parsed.proxies.len())),
    }
}

impl SwiftSubApp {
    pub(super) fn start_scheduler(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut first_tick = true;
            loop {
                let delay = if first_tick {
                    Duration::from_secs(3)
                } else {
                    Duration::from_secs(60)
                };
                first_tick = false;
                cx.background_executor().timer(delay).await;
                let Some(this) = this.upgrade() else {
                    break;
                };
                let due = this.update(cx, |this, _| this.due_subscription_ids());
                this.update(cx, |this, cx| this.refresh_builtin_rules_if_due(cx));
                for id in due {
                    this.update(cx, |this, cx| this.refresh_subscription(id, cx));
                }
            }
        })
        .detach();
    }

    fn due_subscription_ids(&self) -> Vec<i64> {
        let now = swiftsub_store::now_ts();
        self.subscriptions
            .iter()
            .filter(|subscription| {
                subscription.enabled
                    && subscription.url.is_some()
                    && !self.refreshing.contains(&subscription.id)
            })
            .filter(|subscription| {
                if self
                    .retry_after
                    .get(&subscription.id)
                    .is_some_and(|until| now < *until)
                {
                    return false;
                }
                subscription.last_ok_at.is_none_or(|last| {
                    now.saturating_sub(last) >= i64::from(subscription.refresh_minutes.max(15)) * 60
                })
            })
            .map(|subscription| subscription.id)
            .collect()
    }
    pub fn refresh_subscription(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.refreshing.contains(&id) || self.refresh_queue.iter().any(|job| job.id == id) {
            return;
        }
        let Some(subscription) = self
            .subscriptions
            .iter()
            .find(|subscription| subscription.id == id)
            .cloned()
        else {
            return;
        };
        let Some(url) = subscription.url.clone() else {
            self.show_toast("该订阅没有远程地址，无需刷新".to_string(), cx);
            return;
        };
        let cache = CacheValidators {
            etag: subscription.etag.clone(),
            last_modified: subscription.last_modified.clone(),
        };
        let mut options = FetchOptions::default();
        if let Some(user_agent) = subscription.user_agent {
            options.user_agent = user_agent;
        }
        self.fetch_in_background(id, url, subscription.name, cache, options, cx);
    }

    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<i64> = self
            .subscriptions
            .iter()
            .filter(|subscription| subscription.enabled && subscription.url.is_some())
            .map(|subscription| subscription.id)
            .collect();
        if ids.is_empty() {
            self.show_toast("没有可刷新的远程订阅".to_string(), cx);
            return;
        }
        for id in ids {
            self.refresh_subscription(id, cx);
        }
    }

    pub(super) fn fetch_in_background(
        &mut self,
        id: i64,
        url: String,
        name: String,
        cache: CacheValidators,
        options: FetchOptions,
        cx: &mut Context<Self>,
    ) {
        let generation = self.refresh_versions.entry(id).or_default();
        *generation += 1;
        let job = FetchJob {
            id,
            url,
            name,
            cache,
            options,
            generation: *generation,
        };
        self.refresh_queue.retain(|pending| pending.id != id);
        self.refresh_queue.push_back(job);
        self.pump_refresh_queue(cx);
    }

    fn pump_refresh_queue(&mut self, cx: &mut Context<Self>) {
        while let Some(job) = next_job(&mut self.refresh_queue, &self.refreshing) {
            self.start_fetch_job(job, cx);
        }
        cx.notify();
    }

    fn start_fetch_job(&mut self, job: FetchJob, cx: &mut Context<Self>) {
        let FetchJob {
            id,
            url,
            name,
            cache,
            options,
            generation,
        } = job;
        self.refreshing.insert(id);
        cx.notify();

        let store = Arc::clone(&self.store);
        let request_url = url.clone();
        let parsed_sources = self.parsed_sources.clone();
        let cached_payload = self
            .subscriptions
            .iter()
            .find(|sub| sub.id == id)
            .and_then(|sub| sub.payload.clone());
        cx.spawn(async move |this, cx| {
            let (outcome, node_count) = cx
                .background_executor()
                .spawn(async move {
                    fetch_validated_subscription(
                        id,
                        &url,
                        &options,
                        &cache,
                        cached_payload.as_deref(),
                        &parsed_sources,
                    )
                })
                .await;

            let Some(this) = this.upgrade() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.refreshing.remove(&id);
                let current = accepts_result(
                    generation,
                    &request_url,
                    this.refresh_versions.get(&id),
                    this.subscriptions.iter().find(|sub| sub.id == id),
                );
                if !current {
                    this.pump_refresh_queue(cx);
                    return;
                }
                match outcome {
                    Ok(FetchOutcome::Fetched(result)) => {
                        let info = result.info.clone().unwrap_or_default();
                        let traffic = TrafficSnapshot {
                            upload: info.upload,
                            download: info.download,
                            total: info.total,
                            expire_at: info.expire,
                        };
                        {
                            let store = lock_store(&store);
                            if let Err(error) = store.save_fetch_success(
                                id,
                                &result.content,
                                result.etag.as_deref(),
                                result.last_modified.as_deref(),
                                &traffic,
                                node_count,
                            ) {
                                this.show_toast(format!("订阅保存失败，保留上次内容：{error}"), cx);
                                this.pump_refresh_queue(cx);
                                return;
                            }
                            if let Some(hours) = result.update_interval_hours {
                                let minutes = hours.saturating_mul(60).clamp(15, 10_080);
                                if let Err(error) =
                                    store.set_subscription_refresh_minutes(id, minutes)
                                {
                                    this.store_error = Some(format!("刷新间隔保存失败：{error}"));
                                }
                            }
                            if let Err(error) = store.log_refresh(Some(id), true, None) {
                                this.store_error = Some(format!("刷新日志保存失败：{error}"));
                            }
                        }
                        this.failure_counts.remove(&id);
                        this.retry_after.remove(&id);
                        this.show_toast(format!("{name} 已更新，共 {node_count} 个节点"), cx);
                    }
                    Ok(FetchOutcome::NotModified {
                        info,
                        etag,
                        last_modified,
                        update_interval_hours,
                    }) => {
                        let traffic = info.map(|info| TrafficSnapshot {
                            upload: info.upload,
                            download: info.download,
                            total: info.total,
                            expire_at: info.expire,
                        });
                        {
                            let store = lock_store(&store);
                            if let Err(error) = store.save_fetch_not_modified(
                                id,
                                traffic.as_ref(),
                                etag.as_deref(),
                                last_modified.as_deref(),
                            ) {
                                this.show_toast(format!("订阅状态保存失败：{error}"), cx);
                                this.pump_refresh_queue(cx);
                                return;
                            }
                            if let Some(hours) = update_interval_hours
                                && let Err(error) = store.set_subscription_refresh_minutes(
                                    id,
                                    hours.saturating_mul(60).clamp(15, 10_080),
                                )
                            {
                                this.store_error = Some(format!("刷新间隔保存失败：{error}"));
                            }
                            if let Err(error) =
                                store.log_refresh(Some(id), true, Some("not modified"))
                            {
                                this.store_error = Some(format!("刷新日志保存失败：{error}"));
                            }
                        }
                        this.failure_counts.remove(&id);
                        this.retry_after.remove(&id);
                        this.show_toast(format!("{name} 已是最新版本"), cx);
                    }
                    Err(error) => {
                        let message = error.to_string();
                        {
                            let store = lock_store(&store);
                            if let Err(error) = store
                                .save_fetch_error(id, &message)
                                .and_then(|_| store.log_refresh(Some(id), false, Some(&message)))
                            {
                                this.store_error = Some(format!("刷新错误记录保存失败：{error}"));
                            }
                        }
                        let failures = this.failure_counts.entry(id).or_insert(0);
                        *failures = failures.saturating_add(1);
                        let delay_seconds = 3600_i64
                            .saturating_mul(1_i64 << (*failures - 1).min(5))
                            .min(86_400);
                        this.retry_after
                            .insert(id, swiftsub_store::now_ts() + delay_seconds);
                        this.show_toast(format!("{name} 更新失败：{message}"), cx);
                    }
                }
                match lock_store(&store).get_subscription(id) {
                    Ok(Some(subscription)) => {
                        if let Some(current) =
                            this.subscriptions.iter_mut().find(|sub| sub.id == id)
                        {
                            *current = subscription;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => this.store_error = Some(format!("刷新状态读取失败：{error}")),
                }
                this.schedule_refresh_reload(cx);
                this.pump_refresh_queue(cx);
            });
        })
        .detach();
    }

    fn schedule_refresh_reload(&mut self, cx: &mut Context<Self>) {
        if self.refresh_reload_pending {
            return;
        }
        self.refresh_reload_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    this.refresh_reload_pending = false;
                    this.reload_subscriptions(cx);
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/sub", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let start = std::time::Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                start.elapsed() < Duration::from_secs(5),
                                "request timed out"
                            );
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut buffer = [0; 512];
                    let read = stream.read(&mut buffer).unwrap();
                    assert!(read > 0 && request.len() < 16 * 1024);
                    request.extend_from_slice(&buffer[..read]);
                }
                requests.push(String::from_utf8(request).unwrap().to_lowercase());
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nETag: \"fresh\"\r\nLast-Modified: Wed, 25 Nov 2026 00:00:00 GMT\r\nProfile-Update-Interval: 12\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, thread)
    }

    #[test]
    fn not_modified_requires_usable_cache_and_keeps_new_validators() {
        let cache = generation::ParseCache::default();
        let validators = CacheValidators {
            etag: Some("\"old\"".into()),
            last_modified: None,
        };
        let body = "proxies:\n  - {name: test, type: http, server: example.org, port: 443}\n";
        let (url, server) = response_server(vec![(304, ""), (200, body)]);
        let (outcome, count) = fetch_validated_subscription(
            1,
            &url,
            &FetchOptions::default(),
            &validators,
            Some("corrupt cache"),
            &cache,
        );
        assert!(matches!(outcome, Ok(FetchOutcome::Fetched(_))));
        assert_eq!(count, 1);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| !request.contains("if-none-match"))
        );
        let (url, server) = response_server(vec![(304, "")]);
        let (outcome, count) = fetch_validated_subscription(
            1,
            &url,
            &FetchOptions::default(),
            &validators,
            Some(body),
            &cache,
        );
        match outcome.unwrap() {
            FetchOutcome::NotModified {
                etag,
                last_modified,
                update_interval_hours,
                ..
            } => {
                assert_eq!(etag.as_deref(), Some("\"fresh\""));
                assert!(last_modified.is_some());
                assert_eq!(update_interval_hours, Some(12));
            }
            _ => panic!("expected not modified"),
        }
        assert_eq!(count, 1);
        assert!(server.join().unwrap()[0].contains("if-none-match: \"old\""));
        let (url, server) = response_server(vec![(304, ""), (304, "")]);
        let (outcome, _) = fetch_validated_subscription(
            1,
            &url,
            &FetchOptions::default(),
            &validators,
            None,
            &cache,
        );
        assert!(outcome.unwrap_err().contains("没有可用缓存"));
        server.join().unwrap();
    }

    fn job(id: i64) -> FetchJob {
        FetchJob {
            id,
            url: "https://example.org/sub".into(),
            name: "test".into(),
            cache: CacheValidators::default(),
            options: FetchOptions::default(),
            generation: 1,
        }
    }

    #[test]
    fn queue_caps_concurrency_and_waits_for_same_source_to_finish() {
        let mut queue = VecDeque::from([job(1), job(2), job(3)]);
        let mut active = HashSet::from([1]);
        let next = next_job(&mut queue, &active).unwrap();
        assert_eq!(next.id, 2);
        active.insert(next.id);
        assert!(next_job(&mut queue, &active).is_none());
        active.remove(&2);
        assert_eq!(next_job(&mut queue, &active).unwrap().id, 3);
        assert!(next_job(&mut queue, &active).is_none());
        active.remove(&1);
        assert_eq!(next_job(&mut queue, &active).unwrap().id, 1);
    }

    #[test]
    fn edited_deleted_and_disabled_sources_reject_old_results() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .add_subscription(&NewSubscription::url("test", "https://example.org/sub"))
            .unwrap();
        let mut subscription = store.get_subscription(id).unwrap().unwrap();
        assert!(accepts_result(
            1,
            "https://example.org/sub",
            Some(&1),
            Some(&subscription)
        ));
        assert!(!accepts_result(
            1,
            "https://example.org/sub",
            Some(&2),
            Some(&subscription)
        ));
        assert!(!accepts_result(
            1,
            "https://old.example/sub",
            Some(&1),
            Some(&subscription)
        ));
        assert!(!accepts_result(
            1,
            "https://example.org/sub",
            Some(&1),
            None
        ));
        subscription.enabled = false;
        assert!(!accepts_result(
            1,
            "https://example.org/sub",
            Some(&1),
            Some(&subscription)
        ));
    }
}
