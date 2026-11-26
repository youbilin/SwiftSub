use std::collections::HashMap;

use serde_yaml_ng::{Mapping, Value};
use url::Url;

use crate::error::{Error, Result};
use crate::model::Proxy;
use crate::parser::base64_utils::decode_base64_text;

use super::helpers::*;

pub(super) fn parse_ss(rest: &str) -> Result<Proxy> {
    let (body, name) = split_fragment(rest);
    let (body, query) = split_query(body);

    let (userinfo, host_port) = if let Some((userinfo, host_port)) = body.rsplit_once('@') {
        (userinfo.to_string(), host_port.to_string())
    } else {
        let decoded = decode_base64_text(body)?;
        let (userinfo, host_port) = decoded
            .rsplit_once('@')
            .ok_or_else(|| Error::InvalidLink("invalid Shadowsocks payload".into()))?;
        (userinfo.to_string(), host_port.to_string())
    };

    let (method, password) = if userinfo.contains(':') {
        let (method, password) = userinfo
            .split_once(':')
            .ok_or_else(|| Error::InvalidLink("invalid Shadowsocks credentials".into()))?;
        (percent_decode(method), percent_decode(password))
    } else {
        let decoded = decode_base64_text(&userinfo)?;
        let (method, password) = decoded
            .split_once(':')
            .ok_or_else(|| Error::InvalidLink("invalid Shadowsocks credentials".into()))?;
        (method.to_string(), password.to_string())
    };

    let (server, port) = parse_host_port(&host_port)?;
    let display_name = name.unwrap_or_else(|| format!("SS-{server}:{port}"));
    let mut proxy = Proxy::new("ss", display_name, server, port);
    proxy.insert("cipher", method);
    proxy.insert("password", password);
    proxy.insert("udp", true);

    if let Some(query) = query {
        let mut params = HashMap::new();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            let normalized = key.to_lowercase().replace('_', "-");
            if normalized == "plugin" {
                apply_ss_plugin(&mut proxy, &value);
            } else {
                params.insert(normalized, value.into_owned());
            }
        }
        if let Some(uot) = query_optional_bool(&params, &["uot", "udp-over-tcp"])? {
            proxy.insert("udp-over-tcp", uot);
        }
        apply_common_opts(&mut proxy, &params)?;
    }

    Ok(proxy)
}

pub(super) fn apply_ss_plugin(proxy: &mut Proxy, raw: &str) {
    let mut parts = raw.split(';');
    let plugin = parts.next().unwrap_or_default().trim().to_lowercase();
    let mut options = HashMap::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((key, value)) = part.split_once('=') {
            options.insert(
                key.trim().to_lowercase().replace('_', "-"),
                value.trim().to_string(),
            );
        } else {
            options.insert(part.to_lowercase().replace('_', "-"), "true".to_string());
        }
    }

    match plugin.as_str() {
        "obfs-local" | "simple-obfs" | "obfs" => {
            proxy.insert("plugin", "obfs");
            let mut plugin_opts = Mapping::new();
            plugin_opts.insert(
                Value::String("mode".into()),
                Value::String(options.remove("obfs").unwrap_or_else(|| "http".into())),
            );
            if let Some(host) = options.remove("obfs-host")
                && !host.is_empty()
            {
                plugin_opts.insert(Value::String("host".into()), Value::String(host));
            }
            proxy.insert("plugin-opts", Value::Mapping(plugin_opts));
        }
        "v2ray-plugin" => {
            proxy.insert("plugin", "v2ray-plugin");
            let mut plugin_opts = Mapping::new();
            plugin_opts.insert(
                Value::String("mode".into()),
                Value::String(options.remove("mode").unwrap_or_else(|| "websocket".into())),
            );
            if let Some(tls) = options.remove("tls") {
                let enabled = !matches!(tls.trim().to_lowercase().as_str(), "0" | "false" | "no");
                if enabled {
                    plugin_opts.insert(Value::String("tls".into()), Value::Bool(true));
                }
            }
            for key in ["host", "path"] {
                if let Some(value) = options.remove(key)
                    && !value.is_empty()
                {
                    plugin_opts.insert(Value::String(key.into()), Value::String(value));
                }
            }
            if let Some(mux) = options.remove("mux") {
                let enabled = !matches!(mux.trim().to_lowercase().as_str(), "0" | "false" | "no");
                plugin_opts.insert(Value::String("mux".into()), Value::Bool(enabled));
            }
            if options
                .get("skip-cert-verify")
                .or_else(|| options.get("insecure"))
                .or_else(|| options.get("allowinsecure"))
                .is_some_and(|v| {
                    matches!(
                        v.trim().to_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
            {
                plugin_opts.insert(Value::String("skip-cert-verify".into()), Value::Bool(true));
            }
            if let Some(fp) = options
                .remove("fingerprint")
                .or_else(|| options.remove("fp"))
                && !fp.is_empty()
            {
                plugin_opts.insert(Value::String("fingerprint".into()), Value::String(fp));
            }
            proxy.insert("plugin-opts", Value::Mapping(plugin_opts));
        }
        "shadow-tls" => {
            proxy.insert("plugin", "shadow-tls");
            let mut plugin_opts = Mapping::new();
            for (src, dst) in [
                ("host", "host"),
                ("sni", "host"),
                ("password", "password"),
                ("passwd", "password"),
                ("fp", "client-fingerprint"),
                ("client-fingerprint", "client-fingerprint"),
            ] {
                if let Some(value) = options.remove(src)
                    && !value.is_empty()
                    && !plugin_opts.contains_key(Value::String(dst.into()))
                {
                    plugin_opts.insert(Value::String(dst.into()), Value::String(value));
                }
            }
            if let Some(version) = options.remove("version")
                && let Ok(num) = version.parse::<u64>()
            {
                plugin_opts.insert(
                    Value::String("version".into()),
                    Value::Number(serde_yaml_ng::Number::from(num)),
                );
            }
            proxy.insert("plugin-opts", Value::Mapping(plugin_opts));
        }
        "restls" => {
            proxy.insert("plugin", "restls");
            let mut plugin_opts = Mapping::new();
            for (src, dst) in [
                ("host", "host"),
                ("sni", "host"),
                ("password", "password"),
                ("version-hint", "version-hint"),
                ("restls-script", "restls-script"),
                ("fp", "client-fingerprint"),
                ("client-fingerprint", "client-fingerprint"),
            ] {
                if let Some(value) = options.remove(src)
                    && !value.is_empty()
                    && !plugin_opts.contains_key(Value::String(dst.into()))
                {
                    plugin_opts.insert(Value::String(dst.into()), Value::String(value));
                }
            }
            proxy.insert("plugin-opts", Value::Mapping(plugin_opts));
        }
        _ => {}
    }
}

pub(super) fn parse_vmess(rest: &str) -> Result<Proxy> {
    let (body, fragment_name) = split_fragment(rest);
    let (encoded_body, query_str) = split_query(body);
    let decoded = decode_base64_text(encoded_body)?;
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&decoded)
        && json.is_object()
    {
        return parse_vmess_json(&json, fragment_name);
    }
    // Fallback: Shadowrocket URI style `vmess://base64(cipher:uuid@host:port)?remarks=...&obfs=...`
    parse_vmess_shadowrocket(&decoded, query_str, fragment_name)
}

pub(super) fn parse_vmess_json(
    json: &serde_json::Value,
    fragment_name: Option<String>,
) -> Result<Proxy> {
    let server = json_string(json, &["add"])
        .ok_or(Error::MissingField("add"))?
        .trim_matches(['[', ']'])
        .to_string();
    let port = json_u64(json, &["port"]).ok_or(Error::MissingField("port"))?;
    let port = u16::try_from(port).map_err(|_| Error::InvalidPort(port.to_string()))?;
    if port == 0 {
        return Err(Error::InvalidPort(port.to_string()));
    }

    let name = json_string(json, &["ps"])
        .or(fragment_name)
        .unwrap_or_else(|| format!("VMess-{server}:{port}"));
    let mut proxy = Proxy::new("vmess", name, server, port);

    let uuid = json_string(json, &["id"]).ok_or(Error::MissingField("id"))?;
    let alter_id = json_u64(json, &["aid"]).unwrap_or(0);
    let cipher = json_string(json, &["scy"]).unwrap_or_else(|| "auto".into());

    proxy.insert("uuid", uuid);
    proxy.insert(
        "alterId",
        Value::Number(serde_yaml_ng::Number::from(alter_id)),
    );
    proxy.insert("cipher", cipher);
    proxy.insert("udp", true);

    let tls = json_string(json, &["tls"])
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "tls" | "1" | "true"
            )
        })
        .unwrap_or(false);
    if tls {
        proxy.insert("tls", true);
        if let Some(sni) = json_string(json, &["sni", "serverName", "servername"]) {
            proxy.insert("servername", sni);
        }
        if let Some(alpn) = json_string(json, &["alpn"]) {
            insert_alpn(&mut proxy, &alpn);
        }
        if let Some(fingerprint) = json_string(json, &["fp", "fingerprint", "client-fingerprint"]) {
            proxy.insert("client-fingerprint", fingerprint);
        }
    }

    if json_boolish(json, &["allowInsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(pe) = json_string(json, &["packetEncoding", "packet-encoding"]) {
        proxy.insert("packet-encoding", pe);
    }
    if json_boolish(json, &["tfo", "tcp-fast-open"]) {
        proxy.insert("tfo", true);
    }
    if let Some(ech) = json_string(json, &["ech", "ech-config"]) {
        insert_ech(&mut proxy, &ech);
    }
    if let Some(udp_val) = json.get("udp") {
        if let Some(b) = udp_val.as_bool() {
            proxy.insert("udp", b);
        } else if let Some(s) = udp_val.as_str()
            && matches!(s.trim().to_ascii_lowercase().as_str(), "0" | "false")
        {
            proxy.insert("udp", false);
        }
    }

    let network = json_string(json, &["net"]).unwrap_or_else(|| "tcp".into());
    let header_type = json_string(json, &["type"]).unwrap_or_default();
    let host = json_string(json, &["host"]).unwrap_or_default();
    let path = json_string(json, &["path"]).unwrap_or_default();
    let service_name = json_string(json, &["serviceName", "service-name"]).unwrap_or_default();

    if matches!(network.to_ascii_lowercase().as_str(), "tcp" | "raw")
        && header_type.eq_ignore_ascii_case("http")
    {
        apply_http_transport(&mut proxy, &host, &path);
    } else {
        apply_transport(&mut proxy, &network, &host, &path, &service_name);
    }
    if matches!(network.to_ascii_lowercase().as_str(), "xhttp" | "splithttp") {
        let mut xq = HashMap::new();
        if let Some(mode) = json_string(json, &["mode"]) {
            xq.insert("mode".to_string(), mode);
        }
        if let Some(extra) = json.get("extra") {
            let extra_str = match extra {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(_) => extra.to_string(),
                _ => String::new(),
            };
            if !extra_str.is_empty() {
                xq.insert("extra".to_string(), extra_str);
            }
        }
        if !xq.is_empty() {
            apply_xhttp_query(&mut proxy, &xq)?;
        }
    }

    Ok(proxy)
}

pub(super) fn parse_vmess_shadowrocket(
    decoded: &str,
    query_str: Option<&str>,
    fragment_name: Option<String>,
) -> Result<Proxy> {
    let (userinfo, host_port) = decoded
        .trim()
        .rsplit_once('@')
        .ok_or_else(|| Error::InvalidLink("invalid VMess payload".into()))?;
    let (cipher, uuid) = userinfo
        .split_once(':')
        .ok_or_else(|| Error::InvalidLink("invalid VMess credentials".into()))?;
    let (server, port) = parse_host_port(host_port)?;
    let mut query = HashMap::new();
    if let Some(qs) = query_str {
        for (k, v) in url::form_urlencoded::parse(qs.as_bytes()) {
            query.insert(k.to_lowercase().replace('_', "-"), v.into_owned());
        }
    }
    let name = query_value(&query, &["remarks", "ps"])
        .map(str::to_string)
        .or(fragment_name)
        .unwrap_or_else(|| format!("VMess-{server}:{port}"));
    let mut proxy = Proxy::new("vmess", name, server, port);
    proxy.insert("uuid", uuid.trim());
    proxy.insert("alterId", Value::Number(serde_yaml_ng::Number::from(0u64)));
    proxy.insert(
        "cipher",
        if cipher.trim().is_empty() {
            "auto"
        } else {
            cipher.trim()
        },
    );
    proxy.insert("udp", true);

    if query_bool(&query, &["tls"]) {
        proxy.insert("tls", true);
        if let Some(sni) = query_value(&query, &["peer", "sni", "servername"]) {
            proxy.insert("servername", sni);
        }
        if let Some(alpn) = query_value(&query, &["alpn"]) {
            insert_alpn(&mut proxy, alpn);
        }
        if let Some(fp) = query_value(&query, &["fp", "fingerprint", "client-fingerprint"]) {
            proxy.insert("client-fingerprint", fp);
        }
    }
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    let network = query_value(&query, &["obfs", "type", "network"]).unwrap_or("tcp");
    let host = query_value(&query, &["obfsparam", "host"]).unwrap_or_default();
    let path = query_value(&query, &["path"]).unwrap_or_default();
    let service_name = query_value(&query, &["servicename", "service-name"]).unwrap_or_default();
    let normalized_net = if network.eq_ignore_ascii_case("none") {
        "tcp"
    } else if network.eq_ignore_ascii_case("websocket") {
        "ws"
    } else {
        network
    };
    apply_transport(&mut proxy, normalized_net, host, path, service_name);
    apply_common_opts(&mut proxy, &query)?;
    Ok(proxy)
}

pub(super) fn parse_vless(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("vless://{rest}"))?;
    let server = url_host(&url)?;
    let port = url.port().unwrap_or(443);
    let name = fragment_name(&url).unwrap_or_else(|| format!("VLESS-{server}:{port}"));
    let mut proxy = Proxy::new("vless", name, server, port);

    let uuid = percent_decode(url.username());
    if uuid.is_empty() {
        return Err(Error::MissingField("uuid"));
    }
    proxy.insert("uuid", uuid);
    proxy.insert("udp", true);

    let query = query_map(&url);
    let security = query_value(&query, &["security", "tls"]).unwrap_or("none");
    let reality = apply_reality_opts(&mut proxy, &query)?;
    if security.eq_ignore_ascii_case("tls") || reality {
        proxy.insert("tls", true);
    }

    if let Some(sni) = query_value(&query, &["sni", "peer", "servername"]) {
        proxy.insert("servername", sni.to_string());
    }
    if let Some(fingerprint) = query_value(&query, &["fp", "fingerprint", "client-fingerprint"]) {
        proxy.insert("client-fingerprint", fingerprint.to_string());
    }
    if let Some(flow) = query_value(&query, &["flow"]) {
        proxy.insert("flow", flow.to_string());
    }
    if let Some(encryption) = query_value(&query, &["encryption"])
        && !encryption.eq_ignore_ascii_case("none")
    {
        proxy.insert("encryption", encryption.to_string());
    }
    if let Some(pe) = query_value(&query, &["packetencoding", "packet-encoding"]) {
        proxy.insert("packet-encoding", pe.to_string());
    }
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }
    apply_common_opts(&mut proxy, &query)?;

    let network = query_value(&query, &["type", "network"]).unwrap_or("tcp");
    if !matches!(
        network.to_ascii_lowercase().as_str(),
        "tcp"
            | "raw"
            | "ws"
            | "websocket"
            | "httpupgrade"
            | "grpc"
            | "http"
            | "h2"
            | "xhttp"
            | "splithttp"
    ) {
        return Err(Error::InvalidLink(format!(
            "Mihomo 暂不支持 VLESS 的 {network} 传输，无法转换该节点"
        )));
    }
    let host = query_value(&query, &["host", "xhttp-host"]).unwrap_or_default();
    let path = query_value(&query, &["path", "xhttp-path"]).unwrap_or_default();
    let service_name = query_value(&query, &["servicename", "service-name"]).unwrap_or_default();
    if matches!(network.to_ascii_lowercase().as_str(), "tcp" | "raw")
        && query_value(&query, &["headertype", "header-type"])
            .is_some_and(|value| value.eq_ignore_ascii_case("http"))
    {
        apply_http_transport(&mut proxy, host, path);
    } else {
        apply_transport(&mut proxy, network, host, path, service_name);
    }
    if matches!(network.to_ascii_lowercase().as_str(), "xhttp" | "splithttp") {
        apply_xhttp_query(&mut proxy, &query)?;
    }

    Ok(proxy)
}

pub(super) fn parse_trojan(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("trojan://{rest}"))?;
    let server = url_host(&url)?;
    let port = url.port().unwrap_or(443);
    let name = fragment_name(&url).unwrap_or_else(|| format!("Trojan-{server}:{port}"));
    let mut proxy = Proxy::new("trojan", name, server, port);

    let username = percent_decode(url.username());
    let pass_part = url.password().map(percent_decode).unwrap_or_default();
    let password = if pass_part.is_empty() {
        username
    } else if username.is_empty() {
        pass_part
    } else {
        format!("{username}:{pass_part}")
    };
    if password.is_empty() {
        return Err(Error::MissingField("password"));
    }
    proxy.insert("password", password);
    proxy.insert("udp", true);

    let query = query_map(&url);
    apply_reality_opts(&mut proxy, &query)?;
    if let Some(sni) = query_value(&query, &["sni", "peer", "servername"]) {
        proxy.insert("sni", sni.to_string());
    }
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }
    if let Some(fingerprint) = query_value(&query, &["fp", "fingerprint", "client-fingerprint"]) {
        proxy.insert("client-fingerprint", fingerprint.to_string());
    }
    apply_common_opts(&mut proxy, &query)?;

    let network = query_value(&query, &["type", "network"]).unwrap_or("tcp");
    let host = query_value(&query, &["host", "xhttp-host"]).unwrap_or_default();
    let path = query_value(&query, &["path", "xhttp-path"]).unwrap_or_default();
    let service_name = query_value(&query, &["servicename", "service-name"]).unwrap_or_default();
    if matches!(network.to_ascii_lowercase().as_str(), "tcp" | "raw")
        && query_value(&query, &["headertype", "header-type"])
            .is_some_and(|value| value.eq_ignore_ascii_case("http"))
    {
        apply_http_transport(&mut proxy, host, path);
    } else {
        apply_transport(&mut proxy, network, host, path, service_name);
    }
    if matches!(network.to_ascii_lowercase().as_str(), "xhttp" | "splithttp") {
        apply_xhttp_query(&mut proxy, &query)?;
    }

    Ok(proxy)
}

pub(super) fn parse_hysteria2(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("hysteria2://{rest}"))?;
    let server = url_host(&url)?;
    let port = url.port().unwrap_or(443);
    let name = fragment_name(&url).unwrap_or_else(|| format!("Hysteria2-{server}:{port}"));
    let mut proxy = Proxy::new("hysteria2", name, server, port);

    let username = percent_decode(url.username());
    let password = url.password().map(percent_decode).unwrap_or_default();
    let auth = if password.is_empty() {
        username
    } else if username.is_empty() {
        password
    } else {
        format!("{username}:{password}")
    };
    if auth.is_empty() {
        return Err(Error::MissingField("password"));
    }
    proxy.insert("password", auth);

    let query = query_map(&url);
    if let Some(sni) = query_value(&query, &["sni", "peer", "servername"]) {
        proxy.insert("sni", sni.to_string());
    }
    if query_bool(&query, &["insecure", "allowinsecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(fp) = query_value(&query, &["pinsha256", "pin-sha256", "fingerprint"]) {
        proxy.insert("fingerprint", fp.to_string());
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }
    if let Some(obfs) = query_value(&query, &["obfs"])
        && !obfs.eq_ignore_ascii_case("none")
    {
        proxy.insert("obfs", obfs.to_string());
        if let Some(password) = query_value(&query, &["obfs-password", "obfspassword"]) {
            proxy.insert("obfs-password", password.to_string());
        }
    }
    if let Some(ports) = query_value(&query, &["mport", "ports"]) {
        proxy.insert("ports", ports.to_string());
    }
    if let Some(hop) = query_value(&query, &["hop-interval", "hopinterval"])
        && let Ok(seconds) = hop.trim_end_matches('s').parse::<u64>()
    {
        proxy.insert("hop-interval", seconds);
    }
    if let Some(cwnd) = query_value(&query, &["cwnd"])
        && let Ok(val) = cwnd.parse::<u64>()
    {
        proxy.insert("cwnd", val);
    }
    if let Some(up) = query_value(&query, &["up", "upmbps"]) {
        proxy.insert("up", normalize_hy2_bandwidth(up));
    }
    if let Some(down) = query_value(&query, &["down", "downmbps"]) {
        proxy.insert("down", normalize_hy2_bandwidth(down));
    }
    apply_common_opts(&mut proxy, &query)?;

    Ok(proxy)
}

pub(super) fn parse_tuic(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("tuic://{rest}"))?;
    let server = url_host(&url)?;
    let port = url.port().unwrap_or(443);
    let name = fragment_name(&url).unwrap_or_else(|| format!("TUIC-{server}:{port}"));
    let mut proxy = Proxy::new("tuic", name, server, port);

    let query = query_map(&url);
    let uuid = percent_decode(url.username());
    let password = url.password().map(percent_decode).unwrap_or_default();
    let explicit_token = query_value(&query, &["token"]);
    let is_v4 = query_value(&query, &["version"]) == Some("4");
    if let Some(token) = explicit_token {
        proxy.insert("token", token.to_string());
        if !uuid.is_empty() {
            proxy.insert("uuid", uuid);
        }
    } else if is_v4 && password.is_empty() && !uuid.is_empty() {
        proxy.insert("token", uuid);
    } else {
        if uuid.is_empty() {
            return Err(Error::MissingField("uuid"));
        }
        proxy.insert("uuid", uuid);
        if !password.is_empty() {
            proxy.insert("password", password);
        }
    }
    proxy.insert("udp", true);

    if let Some(value) = query_value(&query, &["congestion-control", "congestioncontrol", "cc"]) {
        proxy.insert("congestion-controller", value.to_string());
    }
    if let Some(value) = query_value(&query, &["udp-relay-mode", "udprelaymode"]) {
        proxy.insert("udp-relay-mode", value.to_string());
    }
    if let Some(sni) = query_value(&query, &["sni", "peer", "servername"]) {
        proxy.insert("sni", sni.to_string());
    }
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(v) = query_optional_bool(&query, &["disable-sni", "disablesni"])? {
        proxy.insert("disable-sni", v);
    }
    if let Some(v) = query_optional_bool(&query, &["reduce-rtt", "reducertt"])? {
        proxy.insert("reduce-rtt", v);
    }
    if let Some(hb) = query_value(&query, &["heartbeat-interval", "heartbeatinterval"])
        && let Ok(ms) = hb.parse::<u64>()
    {
        proxy.insert("heartbeat-interval", ms);
    }
    if let Some(rt) = query_value(&query, &["request-timeout", "requesttimeout"])
        && let Ok(ms) = rt.parse::<u64>()
    {
        proxy.insert("request-timeout", ms);
    }
    if let Some(fp) = query_value(&query, &["fp", "fingerprint", "client-fingerprint"]) {
        proxy.insert("client-fingerprint", fp.to_string());
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }
    apply_common_opts(&mut proxy, &query)?;

    Ok(proxy)
}
