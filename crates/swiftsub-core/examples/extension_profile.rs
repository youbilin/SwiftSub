use swiftsub_core::generator::ListenerSpec;
use swiftsub_core::{
    GenerateOptions, catalog::Preset, generate_profile, model::ChainSpec, normalize::build_nodes,
    parser::parse_content,
};
fn main() {
    let links = "anytls://password@example.org:443?sni=example.org#AnyTLS\nsnell://password@example.org:443?version=3#Snell\nssh://user:password@example.org:22#SSH\nwireguard://AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D@example.org:51820?publickey=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D&address=10.0.0.2%2F32#WG\nss://YWVzLTEyOC1nY206cGFzc3dvcmQ@example.org:8388#SS";
    let parsed = parse_content(links).unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let nodes = build_nodes(parsed.proxies, "extension", "validation");
    let mut options = GenerateOptions {
        preset: Preset::Minimal,
        enabled_modules: Some(vec![]),
        ..Default::default()
    };
    options.chains.push(ChainSpec {
        name: "Three hops".into(),
        hops: vec!["AnyTLS".into(), "SSH".into(), "SS".into()],
        ..ChainSpec::new("", "", "", "", "")
    });
    options.listeners.push(ListenerSpec {
        name: "private-socks".into(),
        port: 19090,
        proxy: "Three hops".into(),
        kind: "socks".into(),
        listen: "127.0.0.1".into(),
        udp: true,
    });
    options.extra_yaml = "log-level: warning".into();
    let generated = generate_profile(&nodes, &options).unwrap();
    std::fs::write(std::env::args().nth(1).expect("OUTPUT"), generated.yaml).unwrap();
}
