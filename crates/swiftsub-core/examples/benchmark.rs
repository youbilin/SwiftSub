use std::time::Instant;
use swiftsub_core::{GenerateOptions, generate_profile, model::Proxy, normalize::build_nodes};
fn main() {
    for count in [100, 1000, 10000] {
        let proxies = (0..count)
            .map(|i| {
                let mut proxy = Proxy::new(
                    "trojan",
                    format!("HK-{i}"),
                    format!("node-{i}.example.org"),
                    443,
                );
                proxy.insert("password", "benchmark-only");
                proxy
            })
            .collect();
        let nodes = build_nodes(proxies, "benchmark", "synthetic");
        let start = Instant::now();
        let profile = generate_profile(&nodes, &GenerateOptions::default()).unwrap();
        println!(
            "nodes={count} elapsed_ms={} yaml_bytes={} groups={} rules={}",
            start.elapsed().as_millis(),
            profile.yaml.len(),
            profile.stats.groups,
            profile.stats.rules
        );
    }
}
