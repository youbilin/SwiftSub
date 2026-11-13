use std::collections::HashMap;

use serde_yaml_ng::{Mapping, Value};
use url::Url;

use crate::error::{Error, Result};
use crate::model::Proxy;

use super::base64_utils::decode_base64_text;
use super::{ImportReportItem, ImportStatus, ParseWarning};

pub struct LinkOutput {
    pub proxies: Vec<Proxy>,
    pub warnings: Vec<ParseWarning>,
    pub report: Vec<ImportReportItem>,
}

pub fn parse_links(input: &str) -> LinkOutput {
    let mut proxies = Vec::new();
    let mut warnings = Vec::new();
    let mut report = Vec::new();

    for (index, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with(';') {
            continue;
        }
        if !line.contains("://") {
            report.push(ImportReportItem {
                index: index + 1,
                name: None,
                protocol: None,
                status: ImportStatus::Rejected,
                messages: vec!["无法识别的节点链接".into()],
            });
            warnings.push(ParseWarning::new(
                Some(index + 1),
                format!("无法识别的节点链接: {}", truncate(line, 80)),
            ));
            continue;
        }

        match parse_link(line) {
            Ok(proxy) => {
                let messages = link_compatibility_warnings(line);
                report.push(ImportReportItem {
                    index: index + 1,
                    name: Some(proxy.name().to_string()),
                    protocol: Some(proxy.kind().to_string()),
                    status: if messages.is_empty() {
                        ImportStatus::Accepted
                    } else {
                        ImportStatus::Warning
                    },
                    messages: messages.clone(),
                });
                for message in messages {
                    warnings.push(ParseWarning::new(Some(index + 1), message));
                }
                proxies.push(proxy);
            }
            Err(error) => {
                let message = error.to_string();
                report.push(ImportReportItem {
                    index: index + 1,
                    name: None,
                    protocol: line.split_once("://").map(|(s, _)| s.to_string()),
                    status: ImportStatus::Rejected,
                    messages: vec![message.clone()],
                });
                warnings.push(ParseWarning::new(Some(index + 1), message));
            }
        }
    }

    LinkOutput {
        proxies,
        warnings,
        report,
    }
}

fn parse_link(line: &str) -> Result<Proxy> {
    let (scheme, _) = line
        .split_once("://")
        .ok_or_else(|| Error::InvalidLink("missing scheme".into()))?;
    let rest = &line[scheme.len() + 3..];

    match scheme.to_lowercase().as_str() {
        "ss" => parse_ss(rest),
        "ssr" => parse_ssr(rest),
        "anytls" | "snell" | "wireguard" | "wg" | "ssh" => parse_extended_proxy(scheme, rest),
        "vmess" => parse_vmess(rest),
        "vless" => parse_vless(rest),
        "trojan" => parse_trojan(rest),
        "hysteria2" | "hy2" => parse_hysteria2(rest),
        "tuic" => parse_tuic(rest),
        "http" | "https" | "socks" | "socks5" | "socks4" => parse_simple_proxy(scheme, rest),
        other => Err(Error::UnsupportedProtocol(other.to_string())),
    }
}

fn parse_ssr(rest: &str) -> Result<Proxy> {
    let decoded = decode_base64_text(rest.trim())?;
    let (body, query) = decoded
        .split_once("/?")
        .or_else(|| decoded.split_once('?'))
        .unwrap_or((&decoded, ""));
    let fields: Vec<_> = body.rsplitn(6, ':').collect();
    if fields.len() != 6 {
        return Err(Error::InvalidLink(
            "SSR payload requires server, port, protocol, cipher, obfs and password".into(),
        ));
    }
    let port = fields[4]
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or(Error::MissingField("port"))?;
    let server = fields[5].trim_matches(['[', ']']);
    let mut values = HashMap::new();
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            values.insert(k, percent_decode(v));
        }
    }
    let decode = |key: &str| -> Result<Option<String>> {
        values.get(key).map(|v| decode_base64_text(v)).transpose()
    };
    let name = decode("remarks")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("SSR-{server}:{port}"));
    let mut proxy = Proxy::new("ssr", name, server, port);
    proxy.insert("password", decode_base64_text(fields[0])?);
    proxy.insert("obfs", fields[1]);
    proxy.insert("cipher", fields[2]);
    proxy.insert("protocol", fields[3]);
    proxy.insert("udp", true);
    if let Some(v) = decode("protoparam")? {
        proxy.insert("protocol-param", v);
    }
    if let Some(v) = decode("obfsparam")? {
        proxy.insert("obfs-param", v);
    }
    if let Some(issue) = proxy.fatal_issue() {
        return Err(Error::InvalidLink(issue));
    }
    Ok(proxy)
}

fn parse_extended_proxy(scheme: &str, rest: &str) -> Result<Proxy> {
    let kind = if scheme.eq_ignore_ascii_case("wg") {
        "wireguard".to_string()
    } else {
        scheme.to_lowercase()
    };
    let (body, fragment) = split_fragment(rest);
    let (authority, query) = split_query(body);
    let authority = if kind == "anytls" && !authority.contains('@') {
        decode_base64_text(authority)?
    } else {
        authority.to_string()
    };
    let url = Url::parse(&format!(
        "neutral://{}{}{}",
        authority,
        query.map(|_| "?").unwrap_or(""),
        query.unwrap_or_default()
    ))?;
    let server = url
        .host_str()
        .ok_or(Error::MissingField("server"))?
        .trim_matches(['[', ']'])
        .to_string();
    let port = url.port().unwrap_or(if kind == "ssh" {
        22
    } else if kind == "wireguard" {
        51820
    } else {
        443
    });
    if port == 0 {
        return Err(Error::MissingField("port"));
    }
    let name = fragment.unwrap_or_else(|| format!("{kind}-{server}:{port}"));
    let mut proxy = Proxy::new(kind.clone(), name, server, port);
    let q = query_map(&url);
    let user = percent_decode(url.username());
    let pass = url.password().map(percent_decode);
    let credential = match &pass {
        Some(pass) if !user.is_empty() => format!("{user}:{pass}"),
        Some(pass) => pass.clone(),
        None => user.clone(),
    };
    match kind.as_str() {
        "anytls" => {
            if query_value(&q, &["security"]).is_some_and(|s| !matches!(s, "tls" | "none"))
                || query_value(&q, &["type", "network"])
                    .is_some_and(|s| !matches!(s, "tcp" | "none"))
                || ["pbk", "sid", "path", "host", "servicename"]
                    .iter()
                    .any(|key| q.contains_key(*key))
            {
                return Err(Error::InvalidLink(
                    "AnyTLS only supports native TLS/TCP".into(),
                ));
            }
            proxy.insert("password", credential);
            proxy.insert("tls", true);
            if let Some(v) = query_value(&q, &["sni", "peer", "servername"]) {
                proxy.insert("sni", v);
            }
            if let Some(v) =
                query_optional_bool(&q, &["insecure", "allowinsecure", "skip-cert-verify"])?
            {
                proxy.insert("skip-cert-verify", v);
            }
            if let Some(v) = query_value(&q, &["fp", "client-fingerprint"]) {
                proxy.insert("client-fingerprint", v);
            }
            if let Some(v) = query_value(&q, &["alpn"]) {
                insert_alpn(&mut proxy, v);
            }
            for field in [
                "idle-session-check-interval",
                "idle-session-timeout",
                "min-idle-session",
            ] {
                if let Some(v) = q.get(field) {
                    let n = v
                        .parse::<u32>()
                        .map_err(|_| Error::InvalidLink(format!("invalid {field}")))?;
                    proxy.insert(field, n);
                }
            }
        }
        "snell" => {
            let psk = if credential.is_empty() {
                query_value(&q, &["psk", "password"])
                    .unwrap_or_default()
                    .to_string()
            } else {
                credential
            };
            proxy.insert("psk", psk);
            let version = query_value(&q, &["version"])
                .unwrap_or("3")
                .parse::<u32>()
                .map_err(|_| Error::InvalidLink("invalid Snell version".into()))?;
            if !(1..=3).contains(&version) {
                return Err(Error::InvalidLink(
                    "Mihomo Snell supports versions 1–3".into(),
                ));
            }
            proxy.insert("version", version);
            if let Some(mode) = query_value(&q, &["obfs"]) {
                if !matches!(mode, "http" | "tls") {
                    return Err(Error::InvalidLink("invalid Snell obfs".into()));
                }
                let mut map = Mapping::new();
                map.insert(Value::String("mode".into()), Value::String(mode.into()));
                if let Some(host) = query_value(&q, &["obfs-host"]) {
                    map.insert(Value::String("host".into()), Value::String(host.into()));
                }
                proxy.insert("obfs-opts", Value::Mapping(map));
            }
            if let Some(v) = query_optional_bool(&q, &["udp", "udp-relay"])? {
                proxy.insert("udp", v);
            }
        }
        "ssh" => {
            if user.is_empty() {
                return Err(Error::MissingField("username"));
            }
            proxy.insert("username", user);
            if let Some(v) = pass.filter(|s| !s.is_empty()) {
                proxy.insert("password", v);
            }
            for field in ["private-key", "private-key-passphrase"] {
                if let Some(v) = q.get(field) {
                    proxy.insert(field, v.clone());
                }
            }
            if let Some(v) = q.get("host-key") {
                proxy.insert(
                    "host-key",
                    Value::Sequence(v.split(',').map(|s| Value::String(s.into())).collect()),
                );
            }
        }
        "wireguard" => {
            let private = query_value(&q, &["privatekey", "private-key"]).unwrap_or(&credential);
            let public = query_value(&q, &["publickey", "public-key", "peer-public-key"])
                .ok_or(Error::MissingField("public-key"))?;
            for key in [private, public] {
                if super::decode_base64(key)?.len() != 32 {
                    return Err(Error::InvalidLink(
                        "WireGuard key must decode to 32 bytes".into(),
                    ));
                }
            }
            proxy.insert("private-key", private);
            proxy.insert("public-key", public);
            if let Some(v) = query_value(&q, &["presharedkey", "pre-shared-key", "preshared-key"]) {
                if super::decode_base64(v)?.len() != 32 {
                    return Err(Error::InvalidLink(
                        "WireGuard preshared key must be 32 bytes".into(),
                    ));
                }
                proxy.insert("pre-shared-key", v);
            }
            let addresses =
                query_value(&q, &["address", "ip"]).ok_or(Error::MissingField("address"))?;
            for address in addresses.split(',') {
                let ip = address
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .parse::<std::net::IpAddr>()
                    .map_err(|_| Error::InvalidLink("invalid WireGuard address".into()))?;
                proxy.insert(if ip.is_ipv4() { "ip" } else { "ipv6" }, ip.to_string());
            }
            if let Some(v) = q.get("reserved") {
                let values = v
                    .split(',')
                    .map(|s| s.parse::<u8>().map(Value::from))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| Error::InvalidLink("reserved requires three bytes".into()))?;
                if values.len() != 3 {
                    return Err(Error::InvalidLink("reserved requires three bytes".into()));
                }
                proxy.insert("reserved", Value::Sequence(values));
            }
            if let Some(v) = q.get("mtu") {
                let mtu = v
                    .parse::<u16>()
                    .ok()
                    .filter(|n| *n >= 576)
                    .ok_or_else(|| Error::InvalidLink("invalid MTU".into()))?;
                proxy.insert("mtu", mtu);
            }
            proxy.insert("udp", true);
        }
        _ => unreachable!(),
    }
    if let Some(issue) = proxy.fatal_issue() {
        return Err(Error::InvalidLink(issue));
    }
    Ok(proxy)
}

fn parse_ss(rest: &str) -> Result<Proxy> {
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
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if key == "plugin" {
                apply_ss_plugin(&mut proxy, &value);
            }
        }
    }

    Ok(proxy)
}

fn apply_ss_plugin(proxy: &mut Proxy, raw: &str) {
    let mut parts = raw.split(';');
    let plugin = parts.next().unwrap_or_default().trim().to_lowercase();
    let mut options = HashMap::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((key, value)) = part.split_once('=') {
            options.insert(key.trim().to_lowercase(), value.trim().to_string());
        } else {
            options.insert(part.to_lowercase(), "true".to_string());
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
            if options.contains_key("tls") {
                plugin_opts.insert(Value::String("tls".into()), Value::Bool(true));
            }
            for key in ["host", "path"] {
                if let Some(value) = options.remove(key)
                    && !value.is_empty()
                {
                    plugin_opts.insert(Value::String(key.into()), Value::String(value));
                }
            }
            if let Some(mux) = options.remove("mux")
                && let Ok(value) = mux.parse::<u64>()
            {
                plugin_opts.insert(
                    Value::String("mux".into()),
                    Value::Number(serde_yaml_ng::Number::from(value)),
                );
            }
            proxy.insert("plugin-opts", Value::Mapping(plugin_opts));
        }
        _ => {}
    }
}

fn parse_vmess(rest: &str) -> Result<Proxy> {
    let (body, fragment_name) = split_fragment(rest);
    let decoded = decode_base64_text(body)?;
    let json: serde_json::Value = serde_json::from_str(&decoded)?;

    let server = json_string(&json, &["add"]).ok_or(Error::MissingField("add"))?;
    let port = json_u64(&json, &["port"]).ok_or(Error::MissingField("port"))?;
    let port = u16::try_from(port).map_err(|_| Error::InvalidPort(port.to_string()))?;
    if port == 0 {
        return Err(Error::InvalidPort(port.to_string()));
    }

    let name = json_string(&json, &["ps"])
        .or(fragment_name)
        .unwrap_or_else(|| format!("VMess-{server}:{port}"));
    let mut proxy = Proxy::new("vmess", name, server.clone(), port);

    let uuid = json_string(&json, &["id"]).ok_or(Error::MissingField("id"))?;
    let alter_id = json_u64(&json, &["aid"]).unwrap_or(0);
    let cipher = json_string(&json, &["scy"]).unwrap_or_else(|| "auto".into());

    proxy.insert("uuid", uuid);
    proxy.insert(
        "alterId",
        Value::Number(serde_yaml_ng::Number::from(alter_id)),
    );
    proxy.insert("cipher", cipher);
    proxy.insert("udp", true);

    let tls = json_string(&json, &["tls"])
        .map(|value| value.eq_ignore_ascii_case("tls"))
        .unwrap_or(false);
    if tls {
        proxy.insert("tls", true);
        if let Some(sni) = json_string(&json, &["sni", "serverName"]) {
            proxy.insert("servername", sni);
        }
        if let Some(alpn) = json_string(&json, &["alpn"]) {
            insert_alpn(&mut proxy, &alpn);
        }
        if let Some(fingerprint) = json_string(&json, &["fp"]) {
            proxy.insert("client-fingerprint", fingerprint);
        }
    }

    if json_boolish(&json, &["allowInsecure"]) {
        proxy.insert("skip-cert-verify", true);
    }

    let network = json_string(&json, &["net"]).unwrap_or_else(|| "tcp".into());
    let header_type = json_string(&json, &["type"]).unwrap_or_default();
    let host = json_string(&json, &["host"]).unwrap_or_default();
    let path = json_string(&json, &["path"]).unwrap_or_default();
    let service_name = json_string(&json, &["serviceName"]).unwrap_or_default();

    if network.eq_ignore_ascii_case("tcp") && header_type.eq_ignore_ascii_case("http") {
        apply_http_transport(&mut proxy, &host, &path);
    } else {
        apply_transport(&mut proxy, &network, &host, &path, &service_name);
    }

    Ok(proxy)
}

fn parse_vless(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("vless://{rest}"))?;
    let server = url
        .host_str()
        .ok_or(Error::MissingField("server"))?
        .to_string();
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
    let public_key = query_value(&query, &["pbk", "publickey", "public-key"]);
    let reality = security.eq_ignore_ascii_case("reality") || public_key.is_some();
    if security.eq_ignore_ascii_case("tls") || reality {
        proxy.insert("tls", true);
    }
    if reality {
        let public_key = public_key.ok_or(Error::MissingField("pbk"))?;
        if !crate::model::is_valid_reality_public_key(public_key) {
            return Err(Error::InvalidLink(
                "REALITY public key must be 32 bytes base64url".into(),
            ));
        }
        let mut reality_opts = Mapping::new();
        reality_opts.insert(
            Value::String("public-key".into()),
            Value::String(public_key.to_string()),
        );
        if let Some(short_id) = query_value(&query, &["sid", "shortid", "short-id"]) {
            if !crate::model::is_valid_reality_short_id(short_id) {
                return Err(Error::InvalidLink(
                    "REALITY short-id must be 0-16 hex characters".into(),
                ));
            }
            reality_opts.insert(
                Value::String("short-id".into()),
                Value::String(short_id.to_string()),
            );
        }
        // Modern Xray REALITY servers expect a ClientHello with X25519-MLKEM768.
        // Keep an explicit opt-out for older servers; pqv is a different (ML-DSA) key.
        let support_mlkem =
            query_optional_bool(&query, &["support-x25519mlkem768", "supportx25519mlkem768"])?
                .unwrap_or(true);
        reality_opts.insert(
            Value::String("support-x25519mlkem768".into()),
            Value::Bool(support_mlkem),
        );
        if query_value(&query, &["pqv", "mldsa65verify", "mldsa65-verify"]).is_some() {
            return Err(Error::InvalidLink(
                "Mihomo 暂不支持 REALITY pqv / mldsa65Verify 公钥验证，无法无损转换；该参数不是 support-x25519mlkem768 开关".into(),
            ));
        }
        proxy.insert("reality-opts", Value::Mapping(reality_opts));
    }

    if let Some(sni) = query_value(&query, &["sni", "peer"]) {
        proxy.insert("servername", sni.to_string());
    }
    if let Some(fingerprint) = query_value(&query, &["fp", "fingerprint"]) {
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
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }

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

fn parse_trojan(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("trojan://{rest}"))?;
    let server = url
        .host_str()
        .ok_or(Error::MissingField("server"))?
        .to_string();
    let port = url.port().unwrap_or(443);
    let name = fragment_name(&url).unwrap_or_else(|| format!("Trojan-{server}:{port}"));
    let mut proxy = Proxy::new("trojan", name, server, port);

    let password = percent_decode(url.username());
    if password.is_empty() {
        return Err(Error::MissingField("password"));
    }
    proxy.insert("password", password);
    proxy.insert("udp", true);

    let query = query_map(&url);
    if let Some(sni) = query_value(&query, &["sni", "peer"]) {
        proxy.insert("servername", sni.to_string());
    }
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }
    if let Some(fingerprint) = query_value(&query, &["fp", "fingerprint"]) {
        proxy.insert("client-fingerprint", fingerprint.to_string());
    }

    let network = query_value(&query, &["type", "network"]).unwrap_or("tcp");
    let host = query_value(&query, &["host"]).unwrap_or_default();
    let path = query_value(&query, &["path"]).unwrap_or_default();
    let service_name = query_value(&query, &["servicename", "service-name"]).unwrap_or_default();
    apply_transport(&mut proxy, network, host, path, service_name);

    Ok(proxy)
}

fn parse_hysteria2(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("hysteria2://{rest}"))?;
    let server = url
        .host_str()
        .ok_or(Error::MissingField("server"))?
        .to_string();
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
    if let Some(sni) = query_value(&query, &["sni", "peer"]) {
        proxy.insert("sni", sni.to_string());
    }
    if query_bool(&query, &["insecure", "allowinsecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(obfs) = query_value(&query, &["obfs"]) {
        proxy.insert("obfs", obfs.to_string());
        if let Some(password) = query_value(&query, &["obfs-password", "obfspassword"]) {
            proxy.insert("obfs-password", password.to_string());
        }
    }
    if let Some(ports) = query_value(&query, &["mport", "ports"]) {
        proxy.insert("ports", ports.to_string());
    }
    if let Some(up) = query_value(&query, &["up", "upmbps"]) {
        proxy.insert("up", up.to_string());
    }
    if let Some(down) = query_value(&query, &["down", "downmbps"]) {
        proxy.insert("down", down.to_string());
    }

    Ok(proxy)
}

fn parse_tuic(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("tuic://{rest}"))?;
    let server = url
        .host_str()
        .ok_or(Error::MissingField("server"))?
        .to_string();
    let port = url.port().unwrap_or(443);
    let name = fragment_name(&url).unwrap_or_else(|| format!("TUIC-{server}:{port}"));
    let mut proxy = Proxy::new("tuic", name, server, port);

    let uuid = percent_decode(url.username());
    let password = url.password().map(percent_decode).unwrap_or_default();
    if uuid.is_empty() {
        return Err(Error::MissingField("uuid"));
    }
    proxy.insert("uuid", uuid);
    if !password.is_empty() {
        proxy.insert("password", password);
    }
    proxy.insert("udp", true);

    let query = query_map(&url);
    if let Some(value) = query_value(&query, &["congestion-control", "congestioncontrol", "cc"]) {
        proxy.insert("congestion-controller", value.to_string());
    }
    if let Some(value) = query_value(&query, &["udp-relay-mode", "udprelaymode"]) {
        proxy.insert("udp-relay-mode", value.to_string());
    }
    if let Some(sni) = query_value(&query, &["sni", "peer"]) {
        proxy.insert("sni", sni.to_string());
    }
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }

    Ok(proxy)
}

fn parse_simple_proxy(scheme: &str, rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("{scheme}://{rest}"))?;
    let server = url
        .host_str()
        .ok_or(Error::MissingField("server"))?
        .to_string();
    let port = match url.port() {
        Some(port) => port,
        None if scheme.eq_ignore_ascii_case("http") => 80,
        None if scheme.eq_ignore_ascii_case("https") => 443,
        None => return Err(Error::MissingField("port")),
    };
    let kind = match scheme.to_lowercase().as_str() {
        "https" | "http" => "http",
        "socks4" => "socks4",
        _ => "socks5",
    };
    let name =
        fragment_name(&url).unwrap_or_else(|| format!("{}-{server}:{port}", kind.to_uppercase()));
    let mut proxy = Proxy::new(kind, name, server, port);

    let username = percent_decode(url.username());
    if !username.is_empty() {
        proxy.insert("username", username);
    }
    if let Some(password) = url.password() {
        let password = percent_decode(password);
        if !password.is_empty() {
            proxy.insert("password", password);
        }
    }
    if scheme.eq_ignore_ascii_case("https") {
        proxy.insert("tls", true);
    }
    if kind == "socks5" {
        proxy.insert("udp", true);
    }

    let query = query_map(&url);
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }

    Ok(proxy)
}

fn apply_transport(proxy: &mut Proxy, network: &str, host: &str, path: &str, service_name: &str) {
    let network = network.trim().to_lowercase();
    if network.is_empty() || matches!(network.as_str(), "tcp" | "raw") {
        proxy.insert("network", "tcp");
        return;
    }

    match network.as_str() {
        "ws" | "websocket" | "httpupgrade" => {
            proxy.insert("network", "ws");
            let mut options = Mapping::new();
            if network == "httpupgrade" {
                options.insert(
                    Value::String("v2ray-http-upgrade".into()),
                    Value::Bool(true),
                );
            }
            if !path.is_empty() {
                options.insert(Value::String("path".into()), Value::String(path.into()));
            }
            if !host.is_empty() {
                let mut headers = Mapping::new();
                headers.insert(Value::String("Host".into()), Value::String(host.into()));
                options.insert(Value::String("headers".into()), Value::Mapping(headers));
            }
            if !options.is_empty() {
                proxy.insert("ws-opts", Value::Mapping(options));
            }
        }
        "grpc" => {
            proxy.insert("network", "grpc");
            let service_name = if service_name.is_empty() {
                path
            } else {
                service_name
            };
            if !service_name.is_empty() {
                let mut options = Mapping::new();
                options.insert(
                    Value::String("grpc-service-name".into()),
                    Value::String(service_name.into()),
                );
                proxy.insert("grpc-opts", Value::Mapping(options));
            }
        }
        "http" => apply_http_transport(proxy, host, path),
        "h2" => {
            proxy.insert("network", "h2");
            let mut options = Mapping::new();
            options.insert(
                Value::String("path".into()),
                Value::String(if path.is_empty() { "/" } else { path }.into()),
            );
            if !host.is_empty() {
                options.insert(
                    Value::String("host".into()),
                    Value::Sequence(vec![Value::String(host.into())]),
                );
            }
            proxy.insert("h2-opts", Value::Mapping(options));
        }
        "xhttp" | "splithttp" => {
            proxy.insert("network", "xhttp");
            let mut options = Mapping::new();
            if !path.is_empty() {
                options.insert(Value::String("path".into()), Value::String(path.into()));
            }
            if !host.is_empty() {
                options.insert(Value::String("host".into()), Value::String(host.into()));
            }
            if !options.is_empty() {
                proxy.insert("xhttp-opts", Value::Mapping(options));
            }
        }
        other => {
            proxy.insert("network", other.to_string());
        }
    }
}

fn apply_http_transport(proxy: &mut Proxy, host: &str, path: &str) {
    proxy.insert("network", "http");
    let mut options = Mapping::new();
    options.insert(
        Value::String("path".into()),
        Value::Sequence(vec![Value::String(
            if path.is_empty() { "/" } else { path }.into(),
        )]),
    );
    if !host.is_empty() {
        let mut headers = Mapping::new();
        headers.insert(
            Value::String("Host".into()),
            Value::Sequence(
                host.split(',')
                    .map(str::trim)
                    .filter(|host| !host.is_empty())
                    .map(|host| Value::String(host.into()))
                    .collect(),
            ),
        );
        options.insert(Value::String("headers".into()), Value::Mapping(headers));
    }
    proxy.insert("http-opts", Value::Mapping(options));
}

fn query_map(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(key, value)| (key.to_lowercase().replace('_', "-"), value.into_owned()))
        .collect()
}

fn query_value<'a>(query: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        query
            .get(*key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    })
}

// These are Mihomo's XHTTP fields, not a blind copy of Xray's JSON object.
const XHTTP_EXTRA_FIELDS: &[(&str, &str)] = &[
    ("host", "host"),
    ("path", "path"),
    ("mode", "mode"),
    ("headers", "headers"),
    ("noGRPCHeader", "no-grpc-header"),
    ("xPaddingBytes", "x-padding-bytes"),
    ("xPaddingObfsMode", "x-padding-obfs-mode"),
    ("xPaddingKey", "x-padding-key"),
    ("xPaddingHeader", "x-padding-header"),
    ("xPaddingPlacement", "x-padding-placement"),
    ("xPaddingMethod", "x-padding-method"),
    ("uplinkHTTPMethod", "uplink-http-method"),
    ("sessionIDPlacement", "session-placement"),
    ("sessionIDKey", "session-key"),
    ("sessionIDTable", "session-table"),
    ("sessionIDLength", "session-length"),
    ("sessionPlacement", "session-placement"),
    ("sessionKey", "session-key"),
    ("sessionTable", "session-table"),
    ("sessionLength", "session-length"),
    ("seqPlacement", "seq-placement"),
    ("seqKey", "seq-key"),
    ("uplinkDataPlacement", "uplink-data-placement"),
    ("uplinkDataKey", "uplink-data-key"),
    ("uplinkChunkSize", "uplink-chunk-size"),
    ("scMaxEachPostBytes", "sc-max-each-post-bytes"),
    ("scMinPostsIntervalMs", "sc-min-posts-interval-ms"),
];
const XHTTP_XMUX_FIELDS: &[(&str, &str)] = &[
    ("maxConcurrency", "max-concurrency"),
    ("maxConnections", "max-connections"),
    ("cMaxReuseTimes", "c-max-reuse-times"),
    ("hMaxRequestTimes", "h-max-request-times"),
    ("hMaxReusableSecs", "h-max-reusable-secs"),
    ("hKeepAlivePeriod", "h-keep-alive-period"),
];

fn apply_xhttp_query(proxy: &mut Proxy, query: &HashMap<String, String>) -> Result<()> {
    let mut options = proxy
        .fields()
        .get("xhttp-opts")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    if let Some(extra) = query_value(query, &["extra"]) {
        let extra: serde_json::Value = serde_json::from_str(extra)
            .map_err(|_| Error::InvalidLink("XHTTP extra 必须是有效的 JSON 对象".into()))?;
        let extra = extra
            .as_object()
            .ok_or_else(|| Error::InvalidLink("XHTTP extra 必须是 JSON 对象".into()))?;
        for (key, value) in extra {
            if key == "xmux" {
                let values = value
                    .as_object()
                    .ok_or_else(|| Error::InvalidLink("XHTTP xmux 必须是 JSON 对象".into()))?;
                let mut reuse = Mapping::new();
                for (name, value) in values {
                    if let Some((_, target)) =
                        XHTTP_XMUX_FIELDS.iter().find(|(source, _)| *source == name)
                    {
                        reuse.insert(
                            Value::String((*target).into()),
                            xhttp_extra_value(target, value)?,
                        );
                    }
                }
                options.insert(
                    Value::String("reuse-settings".into()),
                    Value::Mapping(reuse),
                );
            } else if let Some((_, target)) =
                XHTTP_EXTRA_FIELDS.iter().find(|(source, _)| *source == key)
            {
                let value = xhttp_extra_value(target, value)?;
                // Xray always takes these three fields from the outer settings.
                if !matches!(*target, "host" | "path" | "mode") {
                    options.insert(Value::String((*target).into()), value);
                }
            }
        }
    }
    // Preserve the outer host/path/mode even when extra contains the same fields.
    if let Some(mode) = query_value(query, &["mode", "xhttp-mode"]) {
        options.insert(Value::String("mode".into()), Value::String(mode.into()));
    }
    if !options.contains_key(Value::String("x-padding-bytes".into()))
        && let Some(padding) = query_value(query, &["x-padding-bytes", "xpaddingbytes"])
    {
        options.insert(
            Value::String("x-padding-bytes".into()),
            Value::String(padding.into()),
        );
    }
    if let Some(mode) = options
        .get(Value::String("mode".into()))
        .and_then(Value::as_str)
        && !matches!(mode, "auto" | "packet-up" | "stream-up" | "stream-one")
    {
        return Err(Error::InvalidLink("不支持的 XHTTP mode".into()));
    }
    proxy.insert("xhttp-opts", Value::Mapping(options));
    Ok(())
}

fn xhttp_extra_value(key: &str, value: &serde_json::Value) -> Result<Value> {
    let invalid = || Error::InvalidLink(format!("XHTTP extra 的 {key} 类型无效"));
    match key {
        "no-grpc-header" | "x-padding-obfs-mode" => {
            value.as_bool().map(Value::Bool).ok_or_else(invalid)
        }
        "h-keep-alive-period" => value
            .as_u64()
            .map(|n| Value::Number(n.into()))
            .ok_or_else(invalid),
        "headers" => {
            let headers = value.as_object().ok_or_else(invalid)?;
            let mut result = Mapping::new();
            for (name, value) in headers {
                result.insert(
                    Value::String(name.clone()),
                    Value::String(value.as_str().ok_or_else(invalid)?.into()),
                );
            }
            Ok(Value::Mapping(result))
        }
        _ => match value {
            serde_json::Value::String(text) => Ok(Value::String(text.clone())),
            serde_json::Value::Number(n) if !matches!(key, "mode" | "host" | "path") => {
                Ok(Value::String(n.to_string()))
            }
            _ => Err(invalid()),
        },
    }
}

fn link_compatibility_warnings(line: &str) -> Vec<String> {
    let Ok(url) = Url::parse(line) else {
        return Vec::new();
    };
    if matches!(
        url.scheme(),
        "anytls" | "snell" | "wireguard" | "wg" | "ssh"
    ) {
        let supported: &[&str] = match url.scheme() {
            "anytls" => &[
                "security",
                "type",
                "network",
                "sni",
                "peer",
                "servername",
                "insecure",
                "allowinsecure",
                "skip-cert-verify",
                "fp",
                "client-fingerprint",
                "alpn",
                "idle-session-check-interval",
                "idle-session-timeout",
                "min-idle-session",
            ],
            "snell" => &[
                "psk",
                "password",
                "version",
                "obfs",
                "obfs-host",
                "udp",
                "udp-relay",
            ],
            "ssh" => &["private-key", "private-key-passphrase", "host-key"],
            _ => &[
                "privatekey",
                "private-key",
                "publickey",
                "public-key",
                "peer-public-key",
                "presharedkey",
                "pre-shared-key",
                "preshared-key",
                "address",
                "ip",
                "reserved",
                "mtu",
            ],
        };
        let unsupported: Vec<_> = url
            .query_pairs()
            .map(|(k, _)| k.to_lowercase())
            .filter(|k| !supported.contains(&k.as_str()))
            .collect();
        return if unsupported.is_empty() {
            vec![]
        } else {
            vec![format!("以下分享参数未转换：{}", unsupported.join("、"))]
        };
    }
    if !url.scheme().eq_ignore_ascii_case("vless") {
        return Vec::new();
    }
    let query = query_map(&url);
    let mut warnings = Vec::new();
    if matches!(
        query_value(&query, &["type", "network"])
            .unwrap_or("tcp")
            .to_ascii_lowercase()
            .as_str(),
        "xhttp" | "splithttp"
    ) && let Some(extra) = query_value(&query, &["extra"])
        && let Ok(serde_json::Value::Object(values)) = serde_json::from_str(extra)
    {
        let mut unsupported = Vec::new();
        for (key, value) in &values {
            if key == "xmux" {
                if let Some(xmux) = value.as_object() {
                    for name in xmux.keys() {
                        if !XHTTP_XMUX_FIELDS.iter().any(|(source, _)| *source == name) {
                            unsupported.push(format!("xmux.{name}"));
                        }
                    }
                }
            } else if !XHTTP_EXTRA_FIELDS.iter().any(|(source, _)| *source == key) {
                unsupported.push(key.clone());
            }
        }
        if !unsupported.is_empty() {
            warnings.push(format!(
                "XHTTP extra 中以下参数尚未转换，请检查节点配置：{}",
                unsupported.join("、")
            ));
        }
    }
    warnings
}

fn query_optional_bool(query: &HashMap<String, String>, keys: &[&str]) -> Result<Option<bool>> {
    for key in keys {
        if let Some(value) = query.get(*key) {
            return match value.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Ok(Some(true)),
                "0" | "false" | "no" | "off" => Ok(Some(false)),
                _ => Err(Error::InvalidLink(format!(
                    "{key} 必须是布尔值 true/false 或 1/0"
                ))),
            };
        }
    }
    Ok(None)
}

fn query_bool(query: &HashMap<String, String>, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        query.get(*key).is_some_and(|value| {
            matches!(
                value.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
    })
}

fn fragment_name(url: &Url) -> Option<String> {
    url.fragment()
        .map(percent_decode)
        .filter(|value| !value.trim().is_empty())
}

fn split_fragment(input: &str) -> (&str, Option<String>) {
    match input.rsplit_once('#') {
        Some((body, name)) => (body, Some(percent_decode(name))),
        None => (input, None),
    }
}

fn split_query(input: &str) -> (&str, Option<&str>) {
    match input.split_once('?') {
        Some((body, query)) => (body, Some(query)),
        None => (input, None),
    }
}

fn parse_host_port(input: &str) -> Result<(String, u16)> {
    let input = input.trim().trim_end_matches('/');
    let (host, port) = if let Some(rest) = input.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| Error::InvalidLink("invalid IPv6 address".into()))?;
        let host = &rest[..end];
        let port = rest[end + 1..]
            .strip_prefix(':')
            .ok_or_else(|| Error::InvalidLink("missing port".into()))?;
        (host, port)
    } else {
        input
            .rsplit_once(':')
            .ok_or_else(|| Error::InvalidLink("missing port".into()))?
    };

    let host = percent_decode(host);
    if host.is_empty() {
        return Err(Error::MissingField("server"));
    }
    let port = port
        .trim()
        .parse::<u16>()
        .map_err(|_| Error::InvalidPort(port.to_string()))?;
    if port == 0 {
        return Err(Error::InvalidPort(port.to_string()));
    }
    Ok((host, port))
}

fn percent_decode(input: &str) -> String {
    percent_encoding::percent_decode_str(input)
        .decode_utf8_lossy()
        .to_string()
}

fn insert_alpn(proxy: &mut Proxy, raw: &str) {
    let values: Vec<Value> = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Value::String(value.to_string()))
        .collect();
    if !values.is_empty() {
        proxy.insert("alpn", Value::Sequence(values));
    }
}

fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            serde_json::Value::String(text) if !text.is_empty() => Some(text.clone()),
            serde_json::Value::Number(number) => Some(number.to_string()),
            serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
            _ => None,
        })
    })
}

fn json_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            serde_json::Value::Number(number) => number.as_u64(),
            serde_json::Value::String(text) => text.parse().ok(),
            _ => None,
        })
    })
}

fn json_boolish(value: &serde_json::Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        value.get(*key).is_some_and(|item| match item {
            serde_json::Value::Bool(boolean) => *boolean,
            serde_json::Value::Number(number) => number.as_u64() == Some(1),
            serde_json::Value::String(text) => {
                matches!(text.trim().to_lowercase().as_str(), "1" | "true" | "yes")
            }
            _ => false,
        })
    })
}

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        let truncated: String = input.chars().take(max).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ss_sip002() {
        let proxy = parse_ss("YWVzLTI1Ni1nY206cGFzc3dvcmQ=@1.2.3.4:8388#HK-01").unwrap();
        assert_eq!(proxy.kind(), "ss");
        assert_eq!(proxy.server(), "1.2.3.4");
        assert_eq!(proxy.port(), 8388);
        assert_eq!(proxy.get_str("cipher"), Some("aes-256-gcm"));
        assert_eq!(proxy.get_str("password"), Some("password"));
        assert_eq!(proxy.name(), "HK-01");
    }

    #[test]
    fn parses_ss_legacy() {
        use base64::Engine;
        let payload = base64::engine::general_purpose::STANDARD
            .encode("chacha20-ietf-poly1305:secret@example.com:443");
        let proxy = parse_ss(&format!("{payload}#Legacy")).unwrap();
        assert_eq!(proxy.server(), "example.com");
        assert_eq!(proxy.get_str("cipher"), Some("chacha20-ietf-poly1305"));
        assert_eq!(proxy.name(), "Legacy");
    }

    #[test]
    fn parses_vmess() {
        use base64::Engine;
        let json = r#"{"v":"2","ps":"US-01","add":"us.example.com","port":"443","id":"11111111-2222-3333-4444-555555555555","aid":"0","net":"ws","type":"none","host":"cdn.example.com","path":"/ws","tls":"tls","sni":"us.example.com"}"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(json);
        let proxy = parse_vmess(&encoded).unwrap();
        assert_eq!(proxy.kind(), "vmess");
        assert_eq!(
            proxy.get_str("uuid"),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(proxy.get_str("network"), Some("ws"));
        assert_eq!(proxy.get_bool("tls"), Some(true));
    }

    #[test]
    fn parses_vless_reality() {
        let link = "vless://11111111-2222-3333-4444-555555555555@example.com:443?encryption=none&security=reality&sni=www.microsoft.com&fp=chrome&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=1234&type=ws&path=%2Fws&host=cdn.example.com#JP-Reality";
        let proxy = parse_link(link).unwrap();
        assert_eq!(proxy.kind(), "vless");
        assert_eq!(proxy.get_bool("tls"), Some(true));
        assert_eq!(proxy.get_str("network"), Some("ws"));
        assert_eq!(proxy.get_str("client-fingerprint"), Some("chrome"));
        assert_eq!(proxy.name(), "JP-Reality");
        let reality = proxy.fields().get("reality-opts").unwrap();
        assert!(reality.is_mapping());
    }

    const REALITY_LINK: &str = "vless://11111111-2222-3333-4444-555555555555@example.com:443?security=reality&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=1234";

    #[test]
    fn reality_mlkem_defaults_to_true_and_preserves_explicit_booleans() {
        for (suffix, expected) in [
            ("", true),
            ("&support-x25519mlkem768=true", true),
            ("&support-x25519mlkem768=0", false),
            ("&support_x25519mlkem768=false", false),
            ("&supportX25519MLKEM768=1", true),
            ("&support-x25519mlkem768=OFF", false),
        ] {
            let proxy = parse_link(&format!("{REALITY_LINK}{suffix}")).unwrap();
            let reality = proxy.fields().get("reality-opts").unwrap();
            assert_eq!(
                reality["support-x25519mlkem768"].as_bool(),
                Some(expected),
                "{suffix}"
            );
        }
        assert!(parse_link(&format!("{REALITY_LINK}&support-x25519mlkem768=maybe")).is_err());
        assert!(parse_link(&format!("{REALITY_LINK}&support-x25519mlkem768=")).is_err());
        let plain = parse_link("vless://id@example.com:443?security=tls").unwrap();
        assert!(!plain.fields().contains_key("reality-opts"));
    }

    #[test]
    fn reality_accepts_public_key_aliases_and_empty_alias_fallback() {
        for alias in ["publicKey", "public-key", "public_key"] {
            let link = REALITY_LINK
                .replace("pbk=", &format!("{alias}="))
                .replace("security=reality", "security=none");
            let proxy = parse_link(&link).unwrap();
            assert_eq!(proxy.get_bool("tls"), Some(true));
            assert!(proxy.fields().contains_key("reality-opts"));
        }
        let proxy = parse_link(&format!("{REALITY_LINK}&sni=&peer=fallback.example")).unwrap();
        assert_eq!(proxy.get_str("servername"), Some("fallback.example"));
    }

    #[test]
    fn rejects_pqv_instead_of_silently_dropping_verification() {
        let output = parse_links(&format!(
            "{REALITY_LINK}&pqv=MLDSA65-public-key\ntrojan://pass@example.com:443#valid"
        ));
        assert_eq!(output.proxies.len(), 1);
        assert_eq!(output.warnings.len(), 1);
        assert_eq!(output.warnings[0].line, Some(1));
        assert!(output.warnings[0].message.contains("pqv"));
        // An empty pqv is commonly emitted by clients and means no verification key.
        assert!(parse_link(&format!("{REALITY_LINK}&pqv=")).is_ok());
    }

    #[test]
    fn parses_xray_transport_aliases_and_xhttp_extra() {
        let raw = parse_link(&format!("{REALITY_LINK}&type=raw")).unwrap();
        assert_eq!(raw.get_str("network"), Some("tcp"));
        let http = parse_link(&format!(
            "{REALITY_LINK}&type=raw&headerType=http&host=cdn.example&path=%2Fhttp"
        ))
        .unwrap();
        assert_eq!(http.get_str("network"), Some("http"));
        assert_eq!(
            http.fields()["http-opts"]["path"][0].as_str(),
            Some("/http")
        );
        assert_eq!(
            http.fields()["http-opts"]["headers"]["Host"][0].as_str(),
            Some("cdn.example")
        );
        let upgrade =
            parse_link(&format!("{REALITY_LINK}&type=httpupgrade&path=%2Fupgrade")).unwrap();
        assert_eq!(upgrade.get_str("network"), Some("ws"));
        assert_eq!(
            upgrade.fields()["ws-opts"]["v2ray-http-upgrade"].as_bool(),
            Some(true)
        );
        let extra = r#"{"mode":"packet-up","host":"inner.example","noGRPCHeader":true,"xPaddingBytes":"100-300","scMaxEachPostBytes":1048576,"sessionIDPlacement":"header","sessionIDKey":"X-Session","sessionIDLength":16,"xmux":{"maxConcurrency":"16-32","hKeepAlivePeriod":30},"headers":{"User-Agent":"test"}}"#;
        let mut url = Url::parse(&format!(
            "{REALITY_LINK}&type=splithttp&mode=stream-one&host=outer.example&path=%2Fxhttp"
        ))
        .unwrap();
        url.query_pairs_mut().append_pair("extra", extra);
        let proxy = parse_link(url.as_str()).unwrap();
        assert_eq!(proxy.get_str("network"), Some("xhttp"));
        let opts = &proxy.fields()["xhttp-opts"];
        assert_eq!(opts["mode"].as_str(), Some("stream-one"));
        assert_eq!(opts["host"].as_str(), Some("outer.example"));
        assert_eq!(opts["path"].as_str(), Some("/xhttp"));
        assert_eq!(opts["no-grpc-header"].as_bool(), Some(true));
        assert_eq!(opts["sc-max-each-post-bytes"].as_str(), Some("1048576"));
        assert_eq!(
            opts["reuse-settings"]["max-concurrency"].as_str(),
            Some("16-32")
        );
        assert_eq!(
            opts["reuse-settings"]["h-keep-alive-period"].as_u64(),
            Some(30)
        );
        assert_eq!(opts["headers"]["User-Agent"].as_str(), Some("test"));
        assert_eq!(opts["session-placement"].as_str(), Some("header"));
        assert_eq!(opts["session-key"].as_str(), Some("X-Session"));
        assert_eq!(opts["session-length"].as_str(), Some("16"));
        assert!(parse_links(url.as_str()).warnings.is_empty());
    }

    #[test]
    fn xhttp_reports_unsupported_fields_and_rejects_bad_values() {
        let mut url = Url::parse(&format!("{REALITY_LINK}&type=xhttp")).unwrap();
        url.query_pairs_mut().append_pair(
            "extra",
            r#"{"downloadSettings":{},"futureField":1,"xmux":{"future":1}}"#,
        );
        let output = parse_links(url.as_str());
        assert_eq!(output.proxies.len(), 1);
        assert_eq!(output.warnings.len(), 1);
        for field in ["downloadSettings", "futureField", "xmux.future"] {
            assert!(output.warnings[0].message.contains(field));
        }
        for extra in [
            "[]",
            "{bad",
            r#"{"mode":true}"#,
            r#"{"noGRPCHeader":"yes"}"#,
            r#"{"headers":{"test":1}}"#,
            r#"{"xmux":null}"#,
        ] {
            let mut url = Url::parse(&format!("{REALITY_LINK}&type=xhttp")).unwrap();
            url.query_pairs_mut().append_pair("extra", extra);
            assert!(parse_link(url.as_str()).is_err(), "{extra}");
        }
        assert!(parse_link(&format!("{REALITY_LINK}&type=xhttp&mode=unknown")).is_err());
        for network in ["xdrive", "masque", "unknown"] {
            let output = parse_links(&format!("{REALITY_LINK}&type={network}"));
            assert!(output.proxies.is_empty());
            assert!(output.warnings[0].message.contains(network));
        }
        let proxy = parse_link(&format!(
            "{REALITY_LINK}&type=xhttp&x_padding_bytes=100-300"
        ))
        .unwrap();
        assert_eq!(
            proxy.fields()["xhttp-opts"]["x-padding-bytes"].as_str(),
            Some("100-300")
        );
        let proxy = parse_link(&format!("{REALITY_LINK}&type=xhttp&mode=stream-one")).unwrap();
        assert_eq!(
            proxy.fields()["xhttp-opts"]["mode"].as_str(),
            Some("stream-one")
        );
    }

    #[test]
    fn parses_trojan_and_hysteria2() {
        let trojan =
            parse_trojan("pass@example.com:443?sni=example.com&type=grpc&serviceName=grpc#TROJAN")
                .unwrap();
        assert_eq!(trojan.kind(), "trojan");
        assert_eq!(trojan.get_str("password"), Some("pass"));
        assert_eq!(trojan.get_str("network"), Some("grpc"));

        let hy2 = parse_hysteria2(
            "secret@example.com:8443?sni=example.com&insecure=1&obfs=salamander&obfs-password=abc#HY2",
        )
        .unwrap();
        assert_eq!(hy2.kind(), "hysteria2");
        assert_eq!(hy2.port(), 8443);
        assert_eq!(hy2.get_str("obfs"), Some("salamander"));
        assert_eq!(hy2.get_bool("skip-cert-verify"), Some(true));
    }

    #[test]
    fn rejects_invalid_reality_public_key() {
        let link =
            "vless://11111111-2222-3333-4444-555555555555@example.com:443?security=reality&pbk=abc";
        let error = parse_link(link).unwrap_err();
        assert!(error.to_string().contains("REALITY"));
    }

    #[test]
    fn parses_tuic() {
        let proxy = parse_tuic(
            "11111111-2222-3333-4444-555555555555:pass@example.com:443?congestion_control=bbr&udp_relay_mode=native&alpn=h3#TUIC",
        )
        .unwrap();
        assert_eq!(proxy.kind(), "tuic");
        assert_eq!(proxy.get_str("congestion-controller"), Some("bbr"));
        assert_eq!(proxy.get_str("udp-relay-mode"), Some("native"));
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;
    #[test]
    fn parses_additional_protocols_and_rejects_invalid_parameters() {
        use base64::Engine;
        let raw = "example.org:8388:auth_sha1_v4:aes-256-cfb:tls1.2_ticket_auth:cGFzcw/?remarks=U1NSLXRlc3Q&protoparam=dXNlcg";
        let ssr = format!(
            "ssr://{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
        );
        let p = parse_link(&ssr).unwrap();
        assert_eq!(p.kind(), "ssr");
        assert_eq!(p.get_str("protocol-param"), Some("user"));
        let p=parse_link("anytls://p%40ss@example.org:443?sni=tls.example&insecure=false&idle-session-timeout=30#test").unwrap();
        assert_eq!(p.get_str("password"), Some("p@ss"));
        assert_eq!(p.fields()["skip-cert-verify"].as_bool(), Some(false));
        let p = parse_link("snell://psk@example.org:443?version=3&obfs=tls&obfs-host=cdn.example")
            .unwrap();
        assert_eq!(p.get_str("psk"), Some("psk"));
        let p = parse_link("ssh://user:p%40ss@example.org:22?host-key=ssh-ed25519%20AAA#shell")
            .unwrap();
        assert_eq!(p.get_str("username"), Some("user"));
        let key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D";
        let p=parse_link(&format!("wireguard://{key}@example.org:51820?publickey={key}&address=10.0.0.2%2F32%2Cfd00%3A%3A2%2F128&reserved=1%2C2%2C3&mtu=1280")).unwrap();
        assert_eq!(p.get_str("ip"), Some("10.0.0.2"));
        assert_eq!(p.get_str("ipv6"), Some("fd00::2"));
        for link in [
            "snell://p@example.org?version=4",
            "ssh://user@example.org:22",
            "anytls://p@example.org?security=reality",
            "wireguard://bad@example.org?publickey=bad&address=10.0.0.1",
        ] {
            assert!(parse_link(link).is_err(), "{link}");
        }
        assert!(!link_compatibility_warnings("anytls://p@example.org?unknown=1").is_empty());
    }
}
