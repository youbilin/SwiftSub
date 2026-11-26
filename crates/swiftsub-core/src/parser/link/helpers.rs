use std::collections::HashMap;

use serde_yaml_ng::{Mapping, Value};
use url::Url;

use crate::error::{Error, Result};
use crate::model::Proxy;
use crate::parser::base64_utils::decode_base64_text;

pub(super) fn apply_transport(
    proxy: &mut Proxy,
    network: &str,
    host: &str,
    path: &str,
    service_name: &str,
) {
    let network = network.trim().to_lowercase();
    if network.is_empty() || matches!(network.as_str(), "tcp" | "raw") {
        proxy.insert("network", "tcp");
        return;
    }

    match network.as_str() {
        "ws" | "websocket" | "httpupgrade" => {
            proxy.insert("network", "ws");
            let mut options = Mapping::new();
            let is_upgrade = network == "httpupgrade";
            if is_upgrade {
                options.insert(
                    Value::String("v2ray-http-upgrade".into()),
                    Value::Bool(true),
                );
            }
            let (clean_path, early_data) = if is_upgrade {
                (path.to_string(), None)
            } else {
                split_ws_early_data(path)
            };
            if !clean_path.is_empty() {
                options.insert(Value::String("path".into()), Value::String(clean_path));
            }
            if let Some(ed) = early_data {
                options.insert(
                    Value::String("max-early-data".into()),
                    Value::Number(serde_yaml_ng::Number::from(ed)),
                );
                options.insert(
                    Value::String("early-data-header-name".into()),
                    Value::String("Sec-WebSocket-Protocol".into()),
                );
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

pub(super) fn apply_http_transport(proxy: &mut Proxy, host: &str, path: &str) {
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

pub(super) fn query_map(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(key, value)| (key.to_lowercase().replace('_', "-"), value.into_owned()))
        .collect()
}

pub(super) fn query_value<'a>(
    query: &'a HashMap<String, String>,
    keys: &[&str],
) -> Option<&'a str> {
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

pub(super) fn apply_xhttp_query(proxy: &mut Proxy, query: &HashMap<String, String>) -> Result<()> {
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
                // Outer host/path/mode take precedence; fall back to extra when outer omitted them.
                let target_key = Value::String((*target).into());
                if !matches!(*target, "host" | "path" | "mode")
                    || !options.contains_key(&target_key)
                {
                    options.insert(target_key, value);
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

pub(super) fn xhttp_extra_value(key: &str, value: &serde_json::Value) -> Result<Value> {
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

pub(super) fn link_compatibility_warnings(line: &str) -> Vec<String> {
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
                "fingerprint",
                "client-fingerprint",
                "alpn",
                "idle-session-check-interval",
                "idle-session-timeout",
                "min-idle-session",
                "udp",
                "udp-relay",
                "tfo",
                "tcp-fast-open",
                "ech",
                "ech-config",
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
            .map(|(k, _)| k.to_lowercase().replace('_', "-"))
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

pub(super) fn query_optional_bool(
    query: &HashMap<String, String>,
    keys: &[&str],
) -> Result<Option<bool>> {
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

pub(super) fn query_bool(query: &HashMap<String, String>, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        query.get(*key).is_some_and(|value| {
            matches!(
                value.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
    })
}

pub(super) fn fragment_name(url: &Url) -> Option<String> {
    url.fragment()
        .map(percent_decode)
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn split_fragment(input: &str) -> (&str, Option<String>) {
    match input.split_once('#') {
        Some((body, name)) => (body, Some(percent_decode(name))),
        None => (input, None),
    }
}

pub(super) fn split_query(input: &str) -> (&str, Option<&str>) {
    match input.split_once('?') {
        Some((body, query)) => (body, Some(query)),
        None => (input, None),
    }
}

pub(super) fn parse_host_port(input: &str) -> Result<(String, u16)> {
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

pub(super) fn percent_decode(input: &str) -> String {
    percent_encoding::percent_decode_str(input)
        .decode_utf8_lossy()
        .to_string()
}

pub(super) fn insert_alpn(proxy: &mut Proxy, raw: &str) {
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

pub(super) fn url_host(url: &Url) -> Result<String> {
    let host = url
        .host_str()
        .ok_or(Error::MissingField("server"))?
        .trim_matches(['[', ']'])
        .to_string();
    if host.is_empty() {
        return Err(Error::MissingField("server"));
    }
    Ok(host)
}

pub(super) fn apply_reality_opts(
    proxy: &mut Proxy,
    query: &HashMap<String, String>,
) -> Result<bool> {
    let security = query_value(query, &["security", "tls"]).unwrap_or("none");
    let public_key = query_value(query, &["pbk", "publickey", "public-key"]);
    let reality = security.eq_ignore_ascii_case("reality") || public_key.is_some();
    if !reality {
        return Ok(false);
    }
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
    if let Some(short_id) = query_value(query, &["sid", "shortid", "short-id"]) {
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
        query_optional_bool(query, &["support-x25519mlkem768", "supportx25519mlkem768"])?
            .unwrap_or(true);
    reality_opts.insert(
        Value::String("support-x25519mlkem768".into()),
        Value::Bool(support_mlkem),
    );
    if query_value(query, &["pqv", "mldsa65verify", "mldsa65-verify"]).is_some() {
        return Err(Error::InvalidLink(
            "Mihomo 暂不支持 REALITY pqv / mldsa65Verify 公钥验证，无法无损转换；该参数不是 support-x25519mlkem768 开关".into(),
        ));
    }
    proxy.insert("reality-opts", Value::Mapping(reality_opts));
    Ok(true)
}

pub(super) fn apply_common_opts(proxy: &mut Proxy, query: &HashMap<String, String>) -> Result<()> {
    if let Some(tfo) = query_optional_bool(query, &["tfo", "tcp-fast-open"])? {
        proxy.insert("tfo", tfo);
    }
    if let Some(ech) = query_value(query, &["ech", "ech-config"]) {
        insert_ech(proxy, ech);
    }
    if let Some(udp) = query_optional_bool(query, &["udp", "udp-relay"])? {
        proxy.insert("udp", udp);
    }
    Ok(())
}

pub(super) fn insert_ech(proxy: &mut Proxy, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() || matches!(trimmed.to_ascii_lowercase().as_str(), "0" | "false" | "off")
    {
        return;
    }
    let mut ech_opts = Mapping::new();
    ech_opts.insert(Value::String("enable".into()), Value::Bool(true));
    if !matches!(trimmed.to_ascii_lowercase().as_str(), "1" | "true" | "on") {
        ech_opts.insert(
            Value::String("config".into()),
            Value::String(trimmed.to_string()),
        );
    }
    proxy.insert("ech-opts", Value::Mapping(ech_opts));
}

pub(super) fn normalize_hy2_bandwidth(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
        format!("{trimmed} Mbps")
    } else {
        trimmed.to_string()
    }
}

pub(super) fn split_ws_early_data(path: &str) -> (String, Option<u64>) {
    let Some((base, query)) = path.split_once('?') else {
        return (path.to_string(), None);
    };
    let mut ed: Option<u64> = None;
    let mut kept = Vec::new();
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k.eq_ignore_ascii_case("ed")
            && let Ok(val) = v.parse::<u64>()
            && val > 0
        {
            ed = Some(val);
            continue;
        }
        if !pair.is_empty() {
            kept.push(pair);
        }
    }
    if ed.is_none() {
        return (path.to_string(), None);
    }
    let clean = if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    (clean, ed)
}

pub(super) fn decode_base64_user_pass(raw: &str) -> Option<(String, String)> {
    let decoded = decode_base64_text(raw).ok()?;
    let (user, pass) = decoded.split_once(':')?;
    if !user.is_empty() && decoded.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
        Some((user.to_string(), pass.to_string()))
    } else {
        None
    }
}

pub(super) fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            serde_json::Value::String(text) if !text.is_empty() => Some(text.clone()),
            serde_json::Value::Number(number) => Some(number.to_string()),
            serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
            _ => None,
        })
    })
}

pub(super) fn json_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            serde_json::Value::Number(number) => number.as_u64(),
            serde_json::Value::String(text) => text.parse().ok(),
            _ => None,
        })
    })
}

pub(super) fn json_boolish(value: &serde_json::Value, keys: &[&str]) -> bool {
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

pub(super) fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        let truncated: String = input.chars().take(max).collect();
        format!("{truncated}…")
    }
}
