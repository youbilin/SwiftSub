use super::*;
use url::Url;

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
    let upgrade = parse_link(&format!("{REALITY_LINK}&type=httpupgrade&path=%2Fupgrade")).unwrap();
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
    assert_eq!(trojan.get_str("sni"), Some("example.com"));
    assert_eq!(trojan.get_str("servername"), None);
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
fn parses_latest_parameters_and_fixes_mihomo_export_fields() {
    use base64::Engine;

    // 1. IPv6 brackets stripped across URL-based protocols
    let vless_v6 = parse_link(
            "vless://11111111-2222-3333-4444-555555555555@[2001:db8::1]:443?security=tls&type=ws&path=%2Fws%3Fed%3D2048&host=cdn.example&packetEncoding=xudp&tfo=1&ech=AEX+DQ==&udp=0#V6",
        )
        .unwrap();
    assert_eq!(vless_v6.server(), "2001:db8::1");
    assert_eq!(vless_v6.get_str("packet-encoding"), Some("xudp"));
    assert_eq!(vless_v6.get_bool("tfo"), Some(true));
    assert_eq!(vless_v6.get_bool("udp"), Some(false));
    assert_eq!(vless_v6.fields()["ws-opts"]["path"].as_str(), Some("/ws"));
    assert_eq!(
        vless_v6.fields()["ws-opts"]["max-early-data"].as_u64(),
        Some(2048)
    );
    assert_eq!(
        vless_v6.fields()["ws-opts"]["early-data-header-name"].as_str(),
        Some("Sec-WebSocket-Protocol")
    );
    assert_eq!(
        vless_v6.fields()["ech-opts"]["enable"].as_bool(),
        Some(true)
    );
    let roundtrip = Proxy::from_yaml_text(&vless_v6.to_yaml_text().unwrap()).unwrap();
    assert_eq!(roundtrip, vless_v6);

    // 2. XHTTP falls back to extra.host/path/mode when outer omits them
    let mut xhttp_url = Url::parse(&format!("{REALITY_LINK}&type=xhttp")).unwrap();
    xhttp_url.query_pairs_mut().append_pair(
        "extra",
        r#"{"host":"inner.example","path":"/inner","mode":"packet-up"}"#,
    );
    let xhttp = parse_link(xhttp_url.as_str()).unwrap();
    assert_eq!(
        xhttp.fields()["xhttp-opts"]["host"].as_str(),
        Some("inner.example")
    );
    assert_eq!(
        xhttp.fields()["xhttp-opts"]["path"].as_str(),
        Some("/inner")
    );
    assert_eq!(
        xhttp.fields()["xhttp-opts"]["mode"].as_str(),
        Some("packet-up")
    );

    // 3. Trojan: sni (not servername), password with colon, REALITY, IPv6
    let trojan = parse_link(
            "trojan://user:pass@[2001:db8::2]:443?sni=sni.example&security=reality&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=ab12&fp=chrome#TR",
        )
        .unwrap();
    assert_eq!(trojan.server(), "2001:db8::2");
    assert_eq!(trojan.get_str("password"), Some("user:pass"));
    assert_eq!(trojan.get_str("sni"), Some("sni.example"));
    assert_eq!(trojan.get_str("servername"), None);
    assert_eq!(
        trojan.fields()["reality-opts"]["short-id"].as_str(),
        Some("ab12")
    );

    // 4. Hysteria2: obfs=none omitted, Mbps normalized, pinSHA256, alpn, hop-interval, cwnd
    let hy2 = parse_link(
            "hy2://pass@[2001:db8::3]:8443?sni=hy2.example&obfs=none&upmbps=100&down=500%20Mbps&pinSHA256=deadbeef&alpn=h3&hop-interval=30s&cwnd=64#HY2-Latest",
        )
        .unwrap();
    assert_eq!(hy2.server(), "2001:db8::3");
    assert_eq!(hy2.get_str("obfs"), None);
    assert_eq!(hy2.get_str("up"), Some("100 Mbps"));
    assert_eq!(hy2.get_str("down"), Some("500 Mbps"));
    assert_eq!(hy2.get_str("fingerprint"), Some("deadbeef"));
    assert_eq!(hy2.get_u64("hop-interval"), Some(30));
    assert_eq!(hy2.get_u64("cwnd"), Some(64));
    assert_eq!(hy2.fields()["alpn"][0].as_str(), Some("h3"));

    // 5. TUIC v5 latest params + TUIC v4 token
    let tuic = parse_link(
            "tuic://11111111-2222-3333-4444-555555555555:secret@[2001:db8::4]:443?sni=t.example&disable_sni=1&reduce_rtt=1&heartbeat_interval=10000&request_timeout=8000&fp=chrome&alpn=h3#TUIC5",
        )
        .unwrap();
    assert_eq!(tuic.server(), "2001:db8::4");
    assert_eq!(tuic.get_bool("disable-sni"), Some(true));
    assert_eq!(tuic.get_bool("reduce-rtt"), Some(true));
    assert_eq!(tuic.get_u64("heartbeat-interval"), Some(10000));
    assert_eq!(tuic.get_u64("request-timeout"), Some(8000));
    assert_eq!(tuic.get_str("client-fingerprint"), Some("chrome"));
    let tuic_v4 = parse_link("tuic://mytoken@example.com:443?version=4#TUIC4").unwrap();
    assert_eq!(tuic_v4.get_str("token"), Some("mytoken"));
    assert!(tuic_v4.fatal_issue().is_none());

    // 6. Shadowsocks: shadow-tls, v2ray-plugin mux bool, uot, tfo
    let ss = parse_link(
            "ss://YWVzLTI1Ni1nY206cGFzcw==@example.com:443?plugin=shadow-tls%3Bhost%3Dtls.example%3Bpassword%3Dstpass%3Bversion%3D3&uot=1&tfo=1#SS-STLS",
        )
        .unwrap();
    assert_eq!(ss.get_str("plugin"), Some("shadow-tls"));
    assert_eq!(
        ss.fields()["plugin-opts"]["host"].as_str(),
        Some("tls.example")
    );
    assert_eq!(ss.fields()["plugin-opts"]["version"].as_u64(), Some(3));
    assert_eq!(ss.get_bool("udp-over-tcp"), Some(true));
    assert_eq!(ss.get_bool("tfo"), Some(true));

    let ss_v2 = parse_link(
            "ss://YWVzLTI1Ni1nY206cGFzcw==@example.com:443?plugin=v2ray-plugin%3Bmode%3Dwebsocket%3Btls%3Bmux%3D0%3Bskip-cert-verify%3D1#SS-V2",
        )
        .unwrap();
    assert_eq!(ss_v2.fields()["plugin-opts"]["mux"].as_bool(), Some(false));
    assert_eq!(
        ss_v2.fields()["plugin-opts"]["skip-cert-verify"].as_bool(),
        Some(true)
    );

    // 7. VMess Shadowrocket URI format & JSON packetEncoding + ws ?ed=
    let sr_payload = base64::engine::general_purpose::STANDARD
        .encode("aes-128-gcm:11111111-2222-3333-4444-555555555555@sr.example.com:443");
    let sr_link = format!(
        "vmess://{sr_payload}?remarks=SR-Node&obfs=websocket&path=%2Fws%3Fed%3D2048&obfsParam=cdn.sr.example&tls=1&peer=sni.sr.example&allowInsecure=1"
    );
    let sr_vmess = parse_link(&sr_link).unwrap();
    assert_eq!(sr_vmess.name(), "SR-Node");
    assert_eq!(sr_vmess.server(), "sr.example.com");
    assert_eq!(sr_vmess.get_str("servername"), Some("sni.sr.example"));
    assert_eq!(sr_vmess.get_bool("skip-cert-verify"), Some(true));
    assert_eq!(
        sr_vmess.fields()["ws-opts"]["max-early-data"].as_u64(),
        Some(2048)
    );

    // 8. SOCKS5 base64 userinfo + IPv6 + TLS + SNI
    let s5_cred = base64::engine::general_purpose::STANDARD.encode("alice:s3cr3t");
    let s5 = parse_link(&format!(
        "socks5://{s5_cred}@[2001:db8::5]:1080?tls=1&sni=socks.example#S5"
    ))
    .unwrap();
    assert_eq!(s5.server(), "2001:db8::5");
    assert_eq!(s5.get_str("username"), Some("alice"));
    assert_eq!(s5.get_str("password"), Some("s3cr3t"));
    assert_eq!(s5.get_bool("tls"), Some(true));
    assert_eq!(s5.get_str("sni"), Some("socks.example"));
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
