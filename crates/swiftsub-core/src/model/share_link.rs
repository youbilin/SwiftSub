use base64::Engine as _;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_yaml_ng::Value;

use super::Proxy;

fn encode_component(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn format_host(server: &str) -> String {
    if server.contains(':') && !(server.starts_with('[') && server.ends_with(']')) {
        format!("[{server}]")
    } else {
        server.to_string()
    }
}

fn push_param(params: &mut Vec<(String, String)>, key: &str, value: impl Into<String>) {
    let value = value.into();
    if !value.is_empty() {
        params.push((key.to_string(), value));
    }
}

fn format_query(params: &[(String, String)]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        let joined: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, encode_component(v)))
            .collect();
        format!("?{}", joined.join("&"))
    }
}

fn map_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn map_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn alpn_csv(proxy: &Proxy) -> Option<String> {
    let items: Vec<&str> = proxy
        .fields()
        .get("alpn")?
        .as_sequence()?
        .iter()
        .filter_map(Value::as_str)
        .filter(|s| !s.is_empty())
        .collect();
    (!items.is_empty()).then(|| items.join(","))
}

fn append_transport_params(proxy: &Proxy, params: &mut Vec<(String, String)>) {
    let network = proxy.get_str("network").unwrap_or("tcp");
    if network != "tcp" {
        push_param(params, "type", network);
    }
    match network {
        "ws" => {
            if let Some(opts) = proxy.fields().get("ws-opts") {
                if let Some(path) = map_str(opts, "path") {
                    push_param(params, "path", path);
                }
                if let Some(host) = opts
                    .get("headers")
                    .and_then(|h| map_str(h, "Host").or_else(|| map_str(h, "host")))
                {
                    push_param(params, "host", host);
                }
                if map_bool(opts, "v2ray-http-upgrade") == Some(true) {
                    params.retain(|(k, _)| k != "type");
                    push_param(params, "type", "httpupgrade");
                }
            }
        }
        "grpc" => {
            if let Some(opts) = proxy.fields().get("grpc-opts")
                && let Some(service) = map_str(opts, "grpc-service-name")
            {
                push_param(params, "serviceName", service);
            }
        }
        "xhttp" => {
            if let Some(opts) = proxy.fields().get("xhttp-opts") {
                if let Some(path) = map_str(opts, "path") {
                    push_param(params, "path", path);
                }
                if let Some(host) = map_str(opts, "host") {
                    push_param(params, "host", host);
                }
                if let Some(mode) = map_str(opts, "mode") {
                    push_param(params, "mode", mode);
                }
            }
        }
        "h2" => {
            if let Some(opts) = proxy.fields().get("h2-opts") {
                if let Some(path) = map_str(opts, "path") {
                    push_param(params, "path", path);
                }
                if let Some(host) = opts
                    .get("host")
                    .and_then(Value::as_sequence)
                    .and_then(|seq| seq.first())
                    .and_then(Value::as_str)
                {
                    push_param(params, "host", host);
                }
            }
        }
        _ => {}
    }
}

impl Proxy {
    /// Export this proxy node into a standard share URI (`vless://`, `trojan://`,
    /// `hysteria2://`, `ss://`, `vmess://`, `tuic://`, `socks5://`, `http://`).
    /// Returns `None` for proxy types that do not have a standard share URI format.
    pub fn to_share_link(&self) -> Option<String> {
        if !self.uses_server_port() || self.server().is_empty() || self.port() == 0 {
            return None;
        }
        let host = format_host(self.server());
        let port = self.port();
        let frag = encode_component(self.name());

        match self.kind() {
            "vless" => {
                let uuid = self.get_str("uuid")?;
                let mut params = Vec::new();
                if let Some(enc) = self.get_str("encryption") {
                    push_param(&mut params, "encryption", enc);
                } else {
                    push_param(&mut params, "encryption", "none");
                }
                if let Some(flow) = self.get_str("flow") {
                    push_param(&mut params, "flow", flow);
                }
                if let Some(reality) = self.fields().get("reality-opts") {
                    push_param(&mut params, "security", "reality");
                    if let Some(pbk) = map_str(reality, "public-key") {
                        push_param(&mut params, "pbk", pbk);
                    }
                    if let Some(sid) = map_str(reality, "short-id") {
                        push_param(&mut params, "sid", sid);
                    }
                    if let Some(mlkem) = map_bool(reality, "support-x25519mlkem768") {
                        push_param(
                            &mut params,
                            "support-x25519mlkem768",
                            if mlkem { "true" } else { "false" },
                        );
                    }
                } else if self.get_bool("tls") == Some(true) {
                    push_param(&mut params, "security", "tls");
                } else {
                    push_param(&mut params, "security", "none");
                }
                if let Some(sni) = self.get_str("servername").or_else(|| self.get_str("sni")) {
                    push_param(&mut params, "sni", sni);
                }
                if let Some(fp) = self.get_str("client-fingerprint") {
                    push_param(&mut params, "fp", fp);
                }
                if let Some(alpn) = alpn_csv(self) {
                    push_param(&mut params, "alpn", alpn);
                }
                if self.get_bool("skip-cert-verify") == Some(true) {
                    push_param(&mut params, "allowInsecure", "1");
                }
                append_transport_params(self, &mut params);
                Some(format!(
                    "vless://{}@{host}:{port}{}#{frag}",
                    encode_component(uuid),
                    format_query(&params)
                ))
            }
            "trojan" => {
                let password = self.get_str("password")?;
                let mut params = Vec::new();
                if let Some(reality) = self.fields().get("reality-opts") {
                    push_param(&mut params, "security", "reality");
                    if let Some(pbk) = map_str(reality, "public-key") {
                        push_param(&mut params, "pbk", pbk);
                    }
                    if let Some(sid) = map_str(reality, "short-id") {
                        push_param(&mut params, "sid", sid);
                    }
                }
                if let Some(sni) = self.get_str("sni").or_else(|| self.get_str("servername")) {
                    push_param(&mut params, "sni", sni);
                }
                if let Some(fp) = self.get_str("client-fingerprint") {
                    push_param(&mut params, "fp", fp);
                }
                if let Some(alpn) = alpn_csv(self) {
                    push_param(&mut params, "alpn", alpn);
                }
                if self.get_bool("skip-cert-verify") == Some(true) {
                    push_param(&mut params, "allowInsecure", "1");
                }
                append_transport_params(self, &mut params);
                Some(format!(
                    "trojan://{}@{host}:{port}{}#{frag}",
                    encode_component(password),
                    format_query(&params)
                ))
            }
            "hysteria2" => {
                let password = self.get_str("password")?;
                let mut params = Vec::new();
                if let Some(sni) = self.get_str("sni") {
                    push_param(&mut params, "sni", sni);
                }
                if let Some(obfs) = self.get_str("obfs").filter(|o| *o != "none") {
                    push_param(&mut params, "obfs", obfs);
                    if let Some(obfs_pw) = self.get_str("obfs-password") {
                        push_param(&mut params, "obfs-password", obfs_pw);
                    }
                }
                if let Some(ports) = self.get_str("ports") {
                    push_param(&mut params, "mport", ports);
                }
                if let Some(fp) = self.get_str("fingerprint") {
                    push_param(&mut params, "pinSHA256", fp);
                }
                if let Some(alpn) = alpn_csv(self) {
                    push_param(&mut params, "alpn", alpn);
                }
                if self.get_bool("skip-cert-verify") == Some(true) {
                    push_param(&mut params, "insecure", "1");
                }
                Some(format!(
                    "hysteria2://{}@{host}:{port}{}#{frag}",
                    encode_component(password),
                    format_query(&params)
                ))
            }
            "ss" => {
                let cipher = self.get_str("cipher")?;
                let password = self.get_str("password")?;
                let userinfo = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(format!("{cipher}:{password}"));
                Some(format!("ss://{userinfo}@{host}:{port}#{frag}"))
            }
            "vmess" => {
                let uuid = self.get_str("uuid")?;
                let network = self.get_str("network").unwrap_or("tcp");
                let mut net_host = "";
                let mut net_path = "";
                if network == "ws"
                    && let Some(opts) = self.fields().get("ws-opts")
                {
                    net_path = map_str(opts, "path").unwrap_or("");
                    net_host = opts
                        .get("headers")
                        .and_then(|h| map_str(h, "Host").or_else(|| map_str(h, "host")))
                        .unwrap_or("");
                } else if network == "grpc"
                    && let Some(opts) = self.fields().get("grpc-opts")
                {
                    net_path = map_str(opts, "grpc-service-name").unwrap_or("");
                }
                let payload = serde_json::json!({
                    "v": "2",
                    "ps": self.name(),
                    "add": self.server(),
                    "port": port,
                    "id": uuid,
                    "aid": self.get_u64("alterId").unwrap_or(0),
                    "scy": self.get_str("cipher").unwrap_or("auto"),
                    "net": network,
                    "type": "none",
                    "host": net_host,
                    "path": net_path,
                    "tls": if self.get_bool("tls") == Some(true) { "tls" } else { "" },
                    "sni": self.get_str("servername").or_else(|| self.get_str("sni")).unwrap_or(""),
                    "fp": self.get_str("client-fingerprint").unwrap_or(""),
                });
                let encoded = base64::engine::general_purpose::STANDARD.encode(payload.to_string());
                Some(format!("vmess://{encoded}"))
            }
            "tuic" => {
                let uuid = self.get_str("uuid")?;
                let password = self.get_str("password").unwrap_or_default();
                let mut params = Vec::new();
                if let Some(sni) = self.get_str("sni") {
                    push_param(&mut params, "sni", sni);
                }
                if let Some(cc) = self.get_str("congestion-controller") {
                    push_param(&mut params, "congestion_control", cc);
                }
                if let Some(mode) = self.get_str("udp-relay-mode") {
                    push_param(&mut params, "udp_relay_mode", mode);
                }
                if let Some(alpn) = alpn_csv(self) {
                    push_param(&mut params, "alpn", alpn);
                }
                if self.get_bool("skip-cert-verify") == Some(true) {
                    push_param(&mut params, "allow_insecure", "1");
                }
                Some(format!(
                    "tuic://{}:{}@{host}:{port}{}#{frag}",
                    encode_component(uuid),
                    encode_component(password),
                    format_query(&params)
                ))
            }
            "socks5" | "http" => {
                let scheme = if self.kind() == "http" && self.get_bool("tls") == Some(true) {
                    "https"
                } else {
                    self.kind()
                };
                let auth = match (self.get_str("username"), self.get_str("password")) {
                    (Some(u), Some(p)) if !u.is_empty() => {
                        format!("{}:{}@", encode_component(u), encode_component(p))
                    }
                    (Some(u), None) if !u.is_empty() => format!("{}@", encode_component(u)),
                    _ => String::new(),
                };
                Some(format!("{scheme}://{auth}{host}:{port}#{frag}"))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse_content;

    #[test]
    fn roundtrips_common_protocols_via_share_links() {
        let links = [
            "vless://11111111-1111-1111-1111-111111111111@vless.example.org:443?encryption=none&security=reality&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=1234&sni=www.cloudflare.com&fp=chrome&flow=xtls-rprx-vision#VLESS-HK",
            "trojan://secret-pass@trojan.example.org:443?sni=trojan.example.org&alpn=h2,http/1.1#Trojan-JP",
            "hysteria2://hy2-pass@hy2.example.org:443?sni=hy2.example.org&obfs=salamander&obfs-password=opw#Hy2-US",
            "ss://YWVzLTI1Ni1nY206c3MtcGFzcw@ss.example.org:8388#SS-SG",
            "tuic://22222222-2222-2222-2222-222222222222:tuic-pass@tuic.example.org:443?sni=tuic.example.org&congestion_control=bbr&udp_relay_mode=native#TUIC-UK",
        ];
        for original in links {
            let first = parse_content(original).unwrap().proxies.remove(0);
            let exported = first.to_share_link().expect("should export share link");
            let second = parse_content(&exported).unwrap().proxies.remove(0);
            assert_eq!(second.kind(), first.kind());
            assert_eq!(second.name(), first.name());
            assert_eq!(second.server(), first.server());
            assert_eq!(second.port(), first.port());
            assert_eq!(second.fatal_issue(), None);
        }
    }
}
