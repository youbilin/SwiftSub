//! Export SwiftSub normalized proxy nodes and routing groups to standard sing-box JSON (1.11+).

use serde_json::{Map, Value as JsonValue, json};
use serde_yaml_ng::Value as YamlValue;

use crate::error::{Error, Result};
use crate::generator::GenerateOptions;
use crate::model::{Node, Proxy};
use crate::normalize::enabled_nodes;

fn yaml_str<'a>(value: &'a YamlValue, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(YamlValue::as_str)
        .filter(|s| !s.is_empty())
}

fn yaml_bool(value: &YamlValue, key: &str) -> Option<bool> {
    value.get(key).and_then(YamlValue::as_bool)
}

fn alpn_list(proxy: &Proxy) -> Option<Vec<String>> {
    let list: Vec<String> = proxy
        .fields()
        .get("alpn")?
        .as_sequence()?
        .iter()
        .filter_map(YamlValue::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    (!list.is_empty()).then_some(list)
}

fn build_tls_object(proxy: &Proxy, default_enabled: bool) -> Option<JsonValue> {
    let has_reality = proxy.fields().contains_key("reality-opts");
    let enabled = has_reality || proxy.get_bool("tls").unwrap_or(default_enabled);
    if !enabled {
        return None;
    }
    let mut tls = Map::new();
    tls.insert("enabled".into(), json!(true));
    if let Some(sni) = proxy.get_str("servername").or_else(|| proxy.get_str("sni"))
        && !sni.is_empty()
    {
        tls.insert("server_name".into(), json!(sni));
    }
    if proxy.get_bool("skip-cert-verify") == Some(true) {
        tls.insert("insecure".into(), json!(true));
    }
    if let Some(alpn) = alpn_list(proxy) {
        tls.insert("alpn".into(), json!(alpn));
    }
    if let Some(fp) = proxy.get_str("client-fingerprint")
        && !fp.is_empty()
    {
        tls.insert("utls".into(), json!({ "enabled": true, "fingerprint": fp }));
    }
    if let Some(reality) = proxy.fields().get("reality-opts") {
        let mut r = Map::new();
        r.insert("enabled".into(), json!(true));
        if let Some(pbk) = yaml_str(reality, "public-key") {
            r.insert("public_key".into(), json!(pbk));
        }
        if let Some(sid) = yaml_str(reality, "short-id") {
            r.insert("short_id".into(), json!(sid));
        }
        tls.insert("reality".into(), JsonValue::Object(r));
    }
    Some(JsonValue::Object(tls))
}

fn build_transport_object(proxy: &Proxy) -> Option<JsonValue> {
    match proxy.get_str("network").unwrap_or("tcp") {
        "ws" => {
            let mut tr = Map::new();
            let is_upgrade = proxy
                .fields()
                .get("ws-opts")
                .and_then(|o| yaml_bool(o, "v2ray-http-upgrade"))
                == Some(true);
            tr.insert(
                "type".into(),
                json!(if is_upgrade { "httpupgrade" } else { "ws" }),
            );
            if let Some(opts) = proxy.fields().get("ws-opts") {
                if let Some(path) = yaml_str(opts, "path") {
                    tr.insert("path".into(), json!(path));
                }
                if let Some(host) = opts
                    .get("headers")
                    .and_then(|h| yaml_str(h, "Host").or_else(|| yaml_str(h, "host")))
                {
                    if is_upgrade {
                        tr.insert("host".into(), json!(host));
                    } else {
                        tr.insert("headers".into(), json!({ "Host": host }));
                    }
                }
                if let Some(ed) = opts.get("max-early-data").and_then(YamlValue::as_u64) {
                    tr.insert("max_early_data".into(), json!(ed));
                }
                if let Some(edh) = yaml_str(opts, "early-data-header-name") {
                    tr.insert("early_data_header_name".into(), json!(edh));
                }
            }
            Some(JsonValue::Object(tr))
        }
        "grpc" => {
            let mut tr = Map::new();
            tr.insert("type".into(), json!("grpc"));
            if let Some(opts) = proxy.fields().get("grpc-opts")
                && let Some(service) = yaml_str(opts, "grpc-service-name")
            {
                tr.insert("service_name".into(), json!(service));
            }
            Some(JsonValue::Object(tr))
        }
        "h2" | "http" => {
            let mut tr = Map::new();
            tr.insert("type".into(), json!("http"));
            if let Some(opts) = proxy
                .fields()
                .get("h2-opts")
                .or_else(|| proxy.fields().get("http-opts"))
            {
                if let Some(path) = yaml_str(opts, "path") {
                    tr.insert("path".into(), json!(path));
                }
                if let Some(hosts) = opts.get("host").and_then(YamlValue::as_sequence) {
                    let list: Vec<&str> = hosts.iter().filter_map(YamlValue::as_str).collect();
                    if !list.is_empty() {
                        tr.insert("host".into(), json!(list));
                    }
                }
            }
            Some(JsonValue::Object(tr))
        }
        _ => None,
    }
}

/// Convert a single normalized `Proxy` into a sing-box outbound JSON object.
/// Returns `None` for proxy types without a direct sing-box equivalent.
pub fn proxy_to_singbox_outbound(proxy: &Proxy) -> Option<JsonValue> {
    if !proxy.uses_server_port() || proxy.server().is_empty() || proxy.port() == 0 {
        return None;
    }
    let mut out = Map::new();
    out.insert("tag".into(), json!(proxy.name()));
    out.insert("server".into(), json!(proxy.server()));
    out.insert("server_port".into(), json!(proxy.port()));

    if let Some(detour) = proxy.get_str("dialer-proxy")
        && !detour.is_empty()
    {
        out.insert("detour".into(), json!(detour));
    }

    match proxy.kind() {
        "vless" => {
            out.insert("type".into(), json!("vless"));
            out.insert("uuid".into(), json!(proxy.get_str("uuid")?));
            if let Some(flow) = proxy.get_str("flow")
                && !flow.is_empty()
            {
                out.insert("flow".into(), json!(flow));
            }
            if let Some(tls) = build_tls_object(proxy, false) {
                out.insert("tls".into(), tls);
            }
            if let Some(tr) = build_transport_object(proxy) {
                out.insert("transport".into(), tr);
            }
        }
        "trojan" => {
            out.insert("type".into(), json!("trojan"));
            out.insert("password".into(), json!(proxy.get_str("password")?));
            if let Some(tls) = build_tls_object(proxy, true) {
                out.insert("tls".into(), tls);
            }
            if let Some(tr) = build_transport_object(proxy) {
                out.insert("transport".into(), tr);
            }
        }
        "hysteria2" => {
            out.insert("type".into(), json!("hysteria2"));
            out.insert("password".into(), json!(proxy.get_str("password")?));
            if let Some(obfs) = proxy.get_str("obfs").filter(|o| *o != "none") {
                let mut o = Map::new();
                o.insert("type".into(), json!(obfs));
                if let Some(pw) = proxy.get_str("obfs-password") {
                    o.insert("password".into(), json!(pw));
                }
                out.insert("obfs".into(), JsonValue::Object(o));
            }
            if let Some(tls) = build_tls_object(proxy, true) {
                out.insert("tls".into(), tls);
            }
        }
        "tuic" => {
            out.insert("type".into(), json!("tuic"));
            out.insert("uuid".into(), json!(proxy.get_str("uuid")?));
            if let Some(pw) = proxy.get_str("password") {
                out.insert("password".into(), json!(pw));
            }
            if let Some(cc) = proxy.get_str("congestion-controller") {
                out.insert("congestion_control".into(), json!(cc));
            }
            if let Some(mode) = proxy.get_str("udp-relay-mode") {
                out.insert("udp_relay_mode".into(), json!(mode));
            }
            if let Some(tls) = build_tls_object(proxy, true) {
                out.insert("tls".into(), tls);
            }
        }
        "ss" => {
            out.insert("type".into(), json!("shadowsocks"));
            out.insert("method".into(), json!(proxy.get_str("cipher")?));
            out.insert("password".into(), json!(proxy.get_str("password")?));
        }
        "vmess" => {
            out.insert("type".into(), json!("vmess"));
            out.insert("uuid".into(), json!(proxy.get_str("uuid")?));
            out.insert(
                "security".into(),
                json!(proxy.get_str("cipher").unwrap_or("auto")),
            );
            out.insert(
                "alter_id".into(),
                json!(proxy.get_u64("alterId").unwrap_or(0)),
            );
            if let Some(tls) = build_tls_object(proxy, false) {
                out.insert("tls".into(), tls);
            }
            if let Some(tr) = build_transport_object(proxy) {
                out.insert("transport".into(), tr);
            }
        }
        "anytls" => {
            out.insert("type".into(), json!("anytls"));
            out.insert("password".into(), json!(proxy.get_str("password")?));
            if let Some(tls) = build_tls_object(proxy, true) {
                out.insert("tls".into(), tls);
            }
        }
        "socks5" => {
            out.insert("type".into(), json!("socks"));
            out.insert("version".into(), json!("5"));
            if let Some(u) = proxy.get_str("username") {
                out.insert("username".into(), json!(u));
            }
            if let Some(p) = proxy.get_str("password") {
                out.insert("password".into(), json!(p));
            }
        }
        "http" => {
            out.insert("type".into(), json!("http"));
            if let Some(u) = proxy.get_str("username") {
                out.insert("username".into(), json!(u));
            }
            if let Some(p) = proxy.get_str("password") {
                out.insert("password".into(), json!(p));
            }
            if let Some(tls) = build_tls_object(proxy, false) {
                out.insert("tls".into(), tls);
            }
        }
        _ => return None,
    }
    Some(JsonValue::Object(out))
}

/// Generate a complete, ready-to-use sing-box 1.11+ JSON configuration string
/// from the current enabled nodes and basic generation options.
pub fn generate_singbox_json(nodes: &[Node], options: &GenerateOptions) -> Result<String> {
    let active = enabled_nodes(nodes);
    let mut outbounds = Vec::new();
    let mut node_tags = Vec::new();
    for node in &active {
        if let Some(outbound) = proxy_to_singbox_outbound(&node.proxy) {
            node_tags.push(node.name().to_string());
            outbounds.push(outbound);
        }
    }
    if node_tags.is_empty() {
        return Err(Error::InvalidConfig(
            "没有可导出为 sing-box 的启用节点".into(),
        ));
    }

    let mut selector_members = vec!["自动选择".to_string()];
    selector_members.extend(node_tags.iter().cloned());
    selector_members.push("direct".to_string());

    let mut final_outbounds = vec![
        json!({
            "type": "selector",
            "tag": "节点选择",
            "outbounds": selector_members,
            "default": "自动选择"
        }),
        json!({
            "type": "urltest",
            "tag": "自动选择",
            "outbounds": node_tags,
            "url": options.test_url,
            "interval": format!("{}s", options.test_interval.max(60)),
            "tolerance": options.test_tolerance
        }),
    ];
    final_outbounds.extend(outbounds);
    final_outbounds.push(json!({ "type": "direct", "tag": "direct" }));

    let listen = if options.allow_lan {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    let doc = json!({
        "log": { "level": "info", "timestamp": true },
        "dns": {
            "servers": [
                { "tag": "remote", "address": "https://1.1.1.1/dns-query", "detour": "节点选择" },
                { "tag": "local", "address": "223.5.5.5", "detour": "direct" }
            ],
            "final": "remote"
        },
        "inbounds": [
            {
                "type": "mixed",
                "tag": "mixed-in",
                "listen": listen,
                "listen_port": options.mixed_port
            }
        ],
        "outbounds": final_outbounds,
        "route": {
            "rules": [
                { "action": "sniff" },
                { "protocol": "dns", "action": "hijack-dns" },
                { "ip_is_private": true, "outbound": "direct" }
            ],
            "final": "节点选择",
            "auto_detect_interface": true
        }
    });
    serde_json::to_string_pretty(&doc).map_err(|error| Error::InvalidConfig(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::build_nodes;
    use crate::parser::parse_content;

    #[test]
    fn exports_multi_protocol_nodes_to_valid_singbox_json() {
        let input = "\
vless://11111111-1111-1111-1111-111111111111@vless.example.org:443?encryption=none&security=reality&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=1234&sni=www.cloudflare.com&fp=chrome&flow=xtls-rprx-vision#VLESS-HK
trojan://pass@trojan.example.org:443?sni=trojan.example.org#Trojan-JP
hysteria2://hy2@hy2.example.org:443?sni=hy2.example.org&obfs=salamander&obfs-password=opw#Hy2-US
ss://YWVzLTI1Ni1nY206cGFzcw@ss.example.org:8388#SS-SG";
        let parsed = parse_content(input).unwrap();
        let nodes = build_nodes(parsed.proxies, "1", "test");
        let text = generate_singbox_json(&nodes, &GenerateOptions::default()).unwrap();
        let doc: JsonValue = serde_json::from_str(&text).unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        // 2 policy groups + 4 proxies + 1 direct = 7
        assert_eq!(outbounds.len(), 7);
        assert_eq!(outbounds[0]["type"], "selector");
        assert_eq!(outbounds[1]["type"], "urltest");
        assert_eq!(outbounds[2]["type"], "vless");
        assert_eq!(outbounds[2]["tls"]["reality"]["enabled"], true);
        assert_eq!(outbounds[4]["type"], "hysteria2");
        assert_eq!(outbounds[5]["type"], "shadowsocks");
    }
}
