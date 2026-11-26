use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct CachedRules {
    pub payload: Vec<String>,
    pub updated_at: i64,
}

pub(super) fn read_cache(store: &Store, key: &str) -> Option<CachedRules> {
    serde_json::from_str(
        &store
            .get_setting(&format!("builtin-rule-cache:{key}"))
            .ok()??,
    )
    .ok()
}

pub(super) fn save_cache(
    store: &Store,
    key: &str,
    payload: Vec<String>,
) -> Result<CachedRules, String> {
    let cache = CachedRules {
        payload,
        updated_at: swiftsub_store::now_ts(),
    };
    let json = serde_json::to_string(&cache).map_err(|error| error.to_string())?;
    store
        .set_setting(&format!("builtin-rule-cache:{key}"), &json)
        .map_err(|error| error.to_string())?;
    Ok(cache)
}

pub(crate) fn is_builtin_rule(id: &str) -> bool {
    catalog::MODULES
        .iter()
        .any(|module| module.rule_sets.iter().any(|set| set.id == id))
        || rule_catalog::SOURCES.iter().any(|source| {
            source
                .entries
                .iter()
                .any(|entry| format!("{}-{}", source.id, entry.id) == id)
        })
}

fn fetch_many(
    bindings: Vec<RuleSetBinding>,
    options: &FetchOptions,
) -> Vec<(String, Result<Vec<String>, String>)> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..bindings.len().min(4) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(binding) = bindings.get(index) else {
                        break;
                    };
                    let result = fetch_rule_payload(binding, options);
                    results
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .push((SwiftSubApp::workspace_rule_cache_key(binding), result));
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner())
}

impl SwiftSubApp {
    fn installed_builtin_bindings(&self) -> Vec<RuleSetBinding> {
        let mut bindings: Vec<_> = self
            .workspace_rule_choices()
            .into_iter()
            .filter(|choice| choice.active && is_builtin_rule(&choice.binding.id))
            .map(|choice| choice.binding)
            .collect();
        for config in &self.saved_configs {
            let Ok(root) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&config.yaml) else {
                continue;
            };
            let Some(providers) = root
                .get("rule-providers")
                .and_then(serde_yaml_ng::Value::as_mapping)
            else {
                continue;
            };
            for (id, provider) in providers {
                let Some(id) = id.as_str().filter(|id| is_builtin_rule(id)) else {
                    continue;
                };
                let Some(url) = provider.get("url").and_then(serde_yaml_ng::Value::as_str) else {
                    continue;
                };
                bindings.push(RuleSetBinding {
                    id: id.into(),
                    name: id.into(),
                    url: url.into(),
                    behavior: provider
                        .get("behavior")
                        .and_then(serde_yaml_ng::Value::as_str)
                        .unwrap_or("domain")
                        .into(),
                    format: provider
                        .get("format")
                        .and_then(serde_yaml_ng::Value::as_str)
                        .unwrap_or("yaml")
                        .into(),
                    target: "DIRECT".into(),
                    no_resolve: false,
                });
            }
        }
        let mut seen = HashSet::new();
        bindings.retain(|binding| seen.insert(Self::workspace_rule_cache_key(binding)));
        bindings
    }

    pub fn refresh_builtin_rules(&mut self, cx: &mut Context<Self>) {
        if self.workspace.rule_updates_busy {
            return;
        }
        let mut bindings = self.installed_builtin_bindings();
        if bindings.is_empty() {
            // The update action remains useful before the first configuration is created.
            bindings = self
                .workspace_rule_choices()
                .into_iter()
                .filter(|choice| choice.source == "内置规则")
                .map(|choice| choice.binding)
                .collect();
        }
        self.workspace.rule_updates_busy = true;
        self.workspace.rule_updates_error = None;
        cx.notify();
        let store = Arc::clone(&self.store);
        let fetch_opts = self.fetch_options();
        cx.spawn(async move |this, cx| {
            let (updated, errors) = cx
                .background_executor()
                .spawn(async move {
                    let mut updated = Vec::new();
                    let mut errors = Vec::new();
                    for (key, result) in fetch_many(bindings, &fetch_opts) {
                        match result.and_then(|payload| {
                            save_cache(
                                &*store.lock().map_err(|error| error.to_string())?,
                                &key,
                                payload,
                            )
                        }) {
                            Ok(cache) => updated.push((key, cache)),
                            Err(error) => errors.push(error),
                        }
                    }
                    if errors.is_empty() && !updated.is_empty() {
                        let _ = lock_store(&store).set_setting(
                            "builtin-rules-updated-at",
                            &swiftsub_store::now_ts().to_string(),
                        );
                    }
                    (updated, errors)
                })
                .await;
            let Some(this) = this.upgrade() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.workspace.rule_updates_busy = false;
                let count = updated.len();
                for (key, cache) in updated {
                    this.workspace
                        .rule_cache_times
                        .insert(key.clone(), cache.updated_at);
                    this.workspace.rule_content_cache.insert(key, cache.payload);
                }
                if errors.is_empty() {
                    this.workspace.rules_updated_at = Some(swiftsub_store::now_ts());
                    this.show_toast(format!("已更新 {count} 个内置规则集"), cx);
                } else {
                    let message = format!(
                        "已更新 {count} 个规则集，{} 个未完成；保留原有内容。{}",
                        errors.len(),
                        errors[0]
                    );
                    this.workspace.rule_updates_error = Some(message.clone());
                    this.show_toast(message, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn refresh_builtin_rules_if_due(&mut self, cx: &mut Context<Self>) {
        if self
            .workspace
            .rules_updated_at
            .is_some_and(|at| swiftsub_store::now_ts().saturating_sub(at) >= 86400)
            && !self.installed_builtin_bindings().is_empty()
        {
            self.refresh_builtin_rules(cx);
        }
    }

    pub fn workspace_refresh_rule_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binding) = self.workspace.rule_content_binding.clone() else {
            return;
        };
        if self.workspace.rule_content_loading {
            return;
        }
        if let Err(error) = self.workspace_checkpoint_rule_content(cx) {
            self.workspace.rule_content_error = Some(error);
            cx.notify();
            return;
        }
        self.workspace.rule_content_request += 1;
        let request = self.workspace.rule_content_request;
        let key = Self::workspace_rule_cache_key(&binding);
        let store = Arc::clone(&self.store);
        self.workspace.rule_content_loading = true;
        self.workspace.rule_content_error = None;
        cx.notify();
        let fetch_opts = self.fetch_options();
        cx.spawn_in(window, async move |this, cx| {
            let fetch = binding.clone();
            let cache_key = key.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let payload = fetch_rule_payload(&fetch, &fetch_opts)?;
                    save_cache(
                        &*store.lock().map_err(|error| error.to_string())?,
                        &cache_key,
                        payload,
                    )
                })
                .await;
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    if this.workspace.rule_content_request != request
                        || this.workspace.rule_content_id.as_ref() != Some(&binding.id)
                    {
                        return;
                    }
                    this.workspace.rule_content_loading = false;
                    match result {
                        Ok(cache) => {
                            this.workspace
                                .rule_cache_times
                                .insert(key.clone(), cache.updated_at);
                            this.workspace
                                .rule_content_cache
                                .insert(key, cache.payload.clone());
                            let local = this
                                .template_base_options
                                .as_ref()
                                .unwrap_or(&this.options)
                                .rule_set_overrides
                                .get(&binding.id)
                                .cloned();
                            // Updating the upstream preview never replaces an authored body or its draft.
                            this.workspace_show_rule_payload(
                                local.unwrap_or(cache.payload),
                                window,
                                cx,
                            );
                            this.show_toast("规则来源已更新，自定义修改已保留".into(), cx);
                        }
                        Err(error) => this.workspace.rule_content_error = Some(error),
                    }
                    cx.notify();
                })
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refreshed_cache_is_scoped_to_source_and_preserves_authored_rules() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting("authored-rules", "keep my edits")
            .unwrap();
        let binding = RuleSetBinding {
            id: "cn".into(),
            name: "CN".into(),
            url: "https://example.org/cn.yaml".into(),
            behavior: "domain".into(),
            format: "yaml".into(),
            target: "DIRECT".into(),
            no_resolve: false,
        };
        let key = SwiftSubApp::workspace_rule_cache_key(&binding);
        save_cache(&store, &key, vec!["+.first.example".into()]).unwrap();
        save_cache(&store, &key, vec!["+.updated.example".into()]).unwrap();
        assert_eq!(
            read_cache(&store, &key).unwrap().payload,
            ["+.updated.example"]
        );
        let other = RuleSetBinding {
            url: "https://another.example/cn.yaml".into(),
            ..binding
        };
        assert!(read_cache(&store, &SwiftSubApp::workspace_rule_cache_key(&other)).is_none());
        assert_eq!(
            store.get_setting("authored-rules").unwrap().as_deref(),
            Some("keep my edits")
        );
    }
}
