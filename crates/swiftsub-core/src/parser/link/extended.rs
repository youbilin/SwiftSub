use std::collections::HashMap;

use serde_yaml_ng::{Mapping, Value};
use url::Url;

use crate::error::{Error, Result};
use crate::model::Proxy;
use crate::parser::base64_utils::{decode_base64, decode_base64_text};

use super::helpers::*;

pub(super) fn parse_ssr(rest: &str) -> Result<Proxy> {
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

pub(super) fn parse_extended_proxy(scheme: &str, rest: &str) -> Result<Proxy> {
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
    let server = url_host(&url)?;
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
            proxy.insert("udp", true);
            if let Some(v) = query_value(&q, &["sni", "peer", "servername"]) {
                proxy.insert("sni", v);
            }
            if let Some(v) =
                query_optional_bool(&q, &["insecure", "allowinsecure", "skip-cert-verify"])?
            {
                proxy.insert("skip-cert-verify", v);
            }
            if let Some(v) = query_value(&q, &["fp", "fingerprint", "client-fingerprint"]) {
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
            apply_common_opts(&mut proxy, &q)?;
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
                if decode_base64(key)?.len() != 32 {
                    return Err(Error::InvalidLink(
                        "WireGuard key must decode to 32 bytes".into(),
                    ));
                }
            }
            proxy.insert("private-key", private);
            proxy.insert("public-key", public);
            if let Some(v) = query_value(&q, &["presharedkey", "pre-shared-key", "preshared-key"]) {
                if decode_base64(v)?.len() != 32 {
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

pub(super) fn parse_simple_proxy(scheme: &str, rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("{scheme}://{rest}"))?;
    let server = url_host(&url)?;
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

    let raw_user = percent_decode(url.username());
    let raw_pass = url.password().map(percent_decode);
    if raw_pass.is_none()
        && !raw_user.is_empty()
        && !raw_user.contains(':')
        && let Some((u, p)) = decode_base64_user_pass(&raw_user)
    {
        if !u.is_empty() {
            proxy.insert("username", u);
        }
        if !p.is_empty() {
            proxy.insert("password", p);
        }
    } else {
        if !raw_user.is_empty() {
            proxy.insert("username", raw_user);
        }
        if let Some(password) = raw_pass
            && !password.is_empty()
        {
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
    if query_bool(&query, &["tls"])
        || query_value(&query, &["security"]).is_some_and(|v| v.eq_ignore_ascii_case("tls"))
    {
        proxy.insert("tls", true);
    }
    if let Some(sni) = query_value(&query, &["sni", "peer", "servername"]) {
        proxy.insert("sni", sni.to_string());
    }
    if let Some(fp) = query_value(&query, &["fp", "fingerprint", "client-fingerprint"]) {
        proxy.insert("client-fingerprint", fp.to_string());
    }
    if query_bool(&query, &["allowinsecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    apply_common_opts(&mut proxy, &query)?;

    Ok(proxy)
}

pub(super) fn parse_hysteria_v1(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("hysteria://{rest}"))?;
    let server = url_host(&url)?;
    let port = url
        .port()
        .filter(|&p| p > 0)
        .ok_or(Error::MissingField("port"))?;
    let name = fragment_name(&url).unwrap_or_else(|| format!("Hysteria-{server}:{port}"));
    let query = query_map(&url);

    let mut proxy = Proxy::new("hysteria", name, server, port);
    if let Some(auth) = query_value(&query, &["auth"]) {
        proxy.insert("auth-str", auth.to_string());
    }
    if let Some(up) = query_value(&query, &["upmbps", "up"]) {
        proxy.insert("up", normalize_hy2_bandwidth(up));
    }
    if let Some(down) = query_value(&query, &["downmbps", "down"]) {
        proxy.insert("down", normalize_hy2_bandwidth(down));
    }
    if let Some(protocol) = query_value(&query, &["protocol"]) {
        proxy.insert("protocol", protocol.to_string());
    }
    if let Some(obfs) = query_value(&query, &["obfs"]).filter(|v| !v.eq_ignore_ascii_case("none")) {
        proxy.insert("obfs", obfs.to_string());
    }
    if let Some(sni) = query_value(&query, &["peer", "sni", "servername"]) {
        proxy.insert("sni", sni.to_string());
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }
    if query_bool(&query, &["insecure", "allowinsecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    proxy.insert("udp", true);
    apply_common_opts(&mut proxy, &query)?;
    Ok(proxy)
}

pub(super) fn parse_juicity(rest: &str) -> Result<Proxy> {
    let url = Url::parse(&format!("juicity://{rest}"))?;
    let server = url_host(&url)?;
    let port = url
        .port()
        .filter(|&p| p > 0)
        .ok_or(Error::MissingField("port"))?;
    let uuid = percent_decode(url.username());
    let password = url.password().map(percent_decode).unwrap_or_default();
    if uuid.is_empty() || password.is_empty() {
        return Err(Error::InvalidLink(
            "Juicity link requires uuid:password".into(),
        ));
    }
    let name = fragment_name(&url).unwrap_or_else(|| format!("Juicity-{server}:{port}"));
    let query = query_map(&url);

    let mut proxy = Proxy::new("juicity", name, server, port);
    proxy.insert("uuid", uuid);
    proxy.insert("password", password);
    proxy.insert("udp", true);
    if let Some(sni) = query_value(&query, &["sni", "servername", "peer"]) {
        proxy.insert("sni", sni.to_string());
    }
    if let Some(cc) = query_value(&query, &["congestion_control", "congestion-controller"]) {
        proxy.insert("congestion-controller", cc.to_string());
    }
    if let Some(fp) = query_value(&query, &["pinned_certchain_sha256", "fingerprint"]) {
        proxy.insert("fingerprint", fp.to_string());
    }
    if let Some(alpn) = query_value(&query, &["alpn"]) {
        insert_alpn(&mut proxy, alpn);
    }
    if query_bool(&query, &["allow_insecure", "insecure", "skip-cert-verify"]) {
        proxy.insert("skip-cert-verify", true);
    }
    apply_common_opts(&mut proxy, &query)?;
    Ok(proxy)
}
