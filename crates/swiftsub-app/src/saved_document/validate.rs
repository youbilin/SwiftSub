use super::*;

impl SavedDocument {
    pub fn validate(&self) -> Result<(), String> {
        let mut names: HashSet<String> = POLICIES.iter().map(|s| s.to_string()).collect();
        for key in ["proxies", "proxy-groups"] {
            for (index, item) in self.items(key).iter().enumerate() {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{key} 第 {} 项需要文本名称与对象格式", index + 1))?;
                if name.trim() != name
                    || name.is_empty()
                    || name.contains([',', '\n', '\r'])
                    || !names.insert(name.to_string())
                {
                    return Err(format!("名称无效或重复：{name}"));
                }
            }
        }
        let groups = self.items("proxy-groups");
        let mut graph = HashMap::<String, Vec<String>>::new();
        for group in groups {
            let name = text(&group, "name");
            if group
                .get("type")
                .and_then(Value::as_str)
                .is_none_or(|kind| kind.trim().is_empty())
            {
                return Err(format!("策略组「{name}」需要有效类型"));
            }
            let kind = text(&group, "type");
            if !["select", "url-test", "fallback", "load-balance", "relay"].contains(&kind.as_str())
            {
                return Err(format!("策略组「{name}」类型无效：{kind}"));
            }
            for key in ["proxies", "use"] {
                if group.get(key).is_some_and(|value| {
                    value
                        .as_sequence()
                        .is_none_or(|items| items.iter().any(|item| item.as_str().is_none()))
                }) {
                    return Err(format!("策略组「{name}」的 {key} 必须是文本列表"));
                }
            }
            if group
                .get("interval")
                .is_some_and(|value| unsigned(Some(value)).is_none_or(|interval| interval == 0))
            {
                return Err(format!("策略组「{name}」的间隔必须是大于零的整数"));
            }
            if group.get("url").is_some_and(|value| {
                value.as_str().is_none_or(|url| {
                    !url::Url::parse(url).is_ok_and(|url| {
                        matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
                    })
                })
            }) {
                return Err(format!("策略组「{name}」测速链接无效"));
            }
            let members = strings(group.get("proxies"));
            let dynamic = group
                .get("use")
                .and_then(Value::as_sequence)
                .is_some_and(|v| !v.is_empty())
                || [
                    "include-all",
                    "include-all-proxies",
                    "include-all-providers",
                ]
                .iter()
                .any(|k| group.get(*k).and_then(Value::as_bool) == Some(true));
            if members.is_empty() && !dynamic {
                return Err(format!("策略组「{name}」至少需要一个成员"));
            }
            for member in &members {
                if !names.contains(member) {
                    return Err(format!("策略组「{name}」引用不存在的成员：{member}"));
                }
            }
            graph.insert(
                name,
                members
                    .into_iter()
                    .filter(|m| !POLICIES.contains(&m.as_str()))
                    .collect(),
            );
            for provider in strings(group.get("use")) {
                if self
                    .root
                    .get("proxy-providers")
                    .and_then(|v| v.get(&provider))
                    .is_none()
                {
                    return Err(format!("未知节点提供者：{provider}"));
                }
            }
        }
        for node in self.items("proxies") {
            let name = text(&node, "name");
            let proxy = swiftsub_core::Proxy::from_value(node.clone())
                .map_err(|error| format!("节点「{name}」：{error}"))?;
            if let Some(error) = proxy.fatal_issue() {
                return Err(format!("节点「{name}」：{error}"));
            }
            let dialer = text(&node, "dialer-proxy");
            graph.insert(
                text(&node, "name"),
                if dialer.is_empty() || POLICIES.contains(&dialer.as_str()) {
                    Vec::new()
                } else {
                    vec![dialer]
                },
            );
        }
        fn visit(
            n: &str,
            graph: &HashMap<String, Vec<String>>,
            active: &mut HashSet<String>,
            done: &mut HashSet<String>,
        ) -> Result<(), String> {
            if done.contains(n) {
                return Ok(());
            }
            if !active.insert(n.into()) {
                return Err(format!("节点或策略组循环引用：{n}"));
            }
            if let Some(edges) = graph.get(n) {
                for edge in edges {
                    visit(edge, graph, active, done)?;
                }
            }
            active.remove(n);
            done.insert(n.into());
            Ok(())
        }
        let mut done = HashSet::new();
        for n in graph.keys() {
            visit(n, &graph, &mut HashSet::new(), &mut done)?;
        }
        for key in [
            "port",
            "socks-port",
            "redir-port",
            "tproxy-port",
            "mixed-port",
        ] {
            if self
                .root
                .get(key)
                .is_some_and(|value| unsigned(Some(value)).is_none_or(|port| port > 65535))
            {
                return Err(format!("{key} 须为 0–65535 的整数"));
            }
        }
        if self.root.get("rules").is_some_and(|value| {
            value
                .as_sequence()
                .is_none_or(|rules| rules.iter().any(|rule| rule.as_str().is_none()))
        }) {
            return Err("rules 必须是文本规则列表".into());
        }
        let rules = self
            .rules()
            .iter()
            .enumerate()
            .map(|(index, line)| {
                if rule_parts(line).first().is_some_and(|kind| kind.eq_ignore_ascii_case("FINAL")) {
                    return Err(format!("规则第 {} 行「{line}」：Mihomo 使用 MATCH 作为兜底规则，请将 FINAL 改为 MATCH", index + 1));
                }
                swiftsub_core::Rule::parse(line)
                    .map_err(|error| format!("规则第 {} 行「{line}」：{error}", index + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let providers = self
            .root
            .get("rule-providers")
            .and_then(Value::as_mapping)
            .map(|providers| {
                providers
                    .keys()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        swiftsub_core::generator::validate_rule_definitions(
            &rules,
            self.root.get("sub-rules"),
            &names,
            &providers,
        )
        .map_err(|error| error.to_string())?;
        for node in self.items("proxies") {
            let dialer = text(&node, "dialer-proxy");
            if !dialer.is_empty() && !names.contains(&dialer) {
                return Err(format!("节点引用不存在的上游：{dialer}"));
            }
        }
        for listener in self.items("listeners") {
            let proxy = text(&listener, "proxy");
            if !proxy.is_empty() && !names.contains(&proxy) {
                return Err(format!("监听引用不存在的目标：{proxy}"));
            }
        }
        Ok(())
    }
}
