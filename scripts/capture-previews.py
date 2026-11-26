"""Capture clean, high-resolution README preview screenshots using isolated synthetic demo data.

Run this script from a normal interactive Windows PowerShell / Terminal session:
    python scripts/capture-previews.py
"""
import hashlib
import json
import os
import pathlib
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "release" / "swiftsub.exe"
CAPTURE_PS1 = ROOT / "scripts" / "capture-window.ps1"
OUT_DIR = ROOT / "docs" / "assets"


def seed_demo_db(app_dir: pathlib.Path, theme: str = "dark"):
    schema_source = (ROOT / "crates/swiftsub-store/src/lib.rs").read_text(encoding="utf-8")
    schema = re.search(r'const SCHEMA: &str = r#"(.*?)"#;', schema_source, re.S).group(1)
    db_path = app_dir / "swiftsub.db"
    if db_path.exists():
        db_path.unlink()

    demo_nodes_yaml = """proxies:
  - name: "香港 01 · IPLC"
    type: vless
    server: hk01.example.com
    port: 443
    uuid: 11111111-2222-3333-4444-555555555551
    tls: true
    flow: xtls-rprx-vision
    servername: www.microsoft.com
    client-fingerprint: chrome
    reality-opts:
      public-key: jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0
      short-id: abcd0123
  - name: "香港 02 · BGP"
    type: trojan
    server: hk02.example.com
    port: 443
    password: demo-password-hk02
    sni: hk02.example.com
    udp: true
  - name: "日本 东京 · 01"
    type: hysteria2
    server: jp01.example.com
    port: 8443
    password: demo-password-jp01
    sni: jp01.example.com
    up: 100 Mbps
    down: 500 Mbps
  - name: "日本 大阪 · 02"
    type: tuic
    server: jp02.example.com
    port: 8443
    uuid: 11111111-2222-3333-4444-555555555554
    password: demo-password-jp02
    congestion-controller: bbr
    udp-relay-mode: native
    sni: jp02.example.com
  - name: "新加坡 01 · 狮城"
    type: vless
    server: sg01.example.com
    port: 443
    uuid: 11111111-2222-3333-4444-555555555555
    tls: true
    network: ws
    servername: sg01.example.com
    ws-opts:
      path: /ws
  - name: "美国 洛杉矶 · AI 专线"
    type: vless
    server: us01.example.com
    port: 443
    uuid: 11111111-2222-3333-4444-555555555556
    tls: true
    flow: xtls-rprx-vision
    servername: www.apple.com
    client-fingerprint: chrome
    reality-opts:
      public-key: jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0
      short-id: ef456789
  - name: "美国 硅谷 · 02"
    type: vmess
    server: us02.example.com
    port: 443
    uuid: 11111111-2222-3333-4444-555555555557
    alterId: 0
    cipher: auto
    tls: true
    network: grpc
    servername: us02.example.com
    grpc-opts:
      grpc-service-name: demo
  - name: "台湾 台北 · 01"
    type: ss
    server: tw01.example.com
    port: 8388
    cipher: 2022-blake3-aes-128-gcm
    password: demo-ss2022-password
    udp: true
  - name: "德国 法兰克福 · 01"
    type: anytls
    server: de01.example.com
    port: 443
    password: demo-anytls-password
    sni: de01.example.com
  - name: "英国 伦敦 · 01"
    type: wireguard
    server: uk01.example.com
    port: 51820
    ip: 172.16.0.2
    private-key: demo-wg-private-key
    public-key: demo-wg-public-key
    udp: true
"""

    saved_yaml_1 = demo_nodes_yaml + """proxy-groups:
  - { name: 节点选择, type: select, proxies: [自动选择, "香港 01 · IPLC", "日本 东京 · 01", "新加坡 01 · 狮城", "美国 洛杉矶 · AI 专线"] }
  - { name: 自动选择, type: url-test, proxies: ["香港 01 · IPLC", "香港 02 · BGP", "日本 东京 · 01", "新加坡 01 · 狮城"], url: https://www.gstatic.com/generate_204, interval: 300 }
  - { name: AI 服务, type: select, proxies: ["美国 洛杉矶 · AI 专线", "日本 东京 · 01", "新加坡 01 · 狮城", 节点选择] }
  - { name: YouTube, type: select, proxies: [节点选择, "香港 01 · IPLC", "日本 东京 · 01"] }
  - { name: GitHub, type: select, proxies: [节点选择, "日本 东京 · 01", DIRECT] }
  - { name: 广告拦截, type: select, proxies: [REJECT, DIRECT] }
  - { name: 国内直连, type: select, proxies: [DIRECT, 节点选择] }
  - { name: 漏网之鱼, type: select, proxies: [节点选择, 自动选择, DIRECT] }
rule-providers:
  category-ads-all: { type: http, behavior: domain, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geosite/category-ads-all.mrs, interval: 86400 }
  openai: { type: http, behavior: domain, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geosite/openai.mrs, interval: 86400 }
  anthropic: { type: http, behavior: domain, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geosite/anthropic.mrs, interval: 86400 }
  youtube: { type: http, behavior: domain, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geosite/youtube.mrs, interval: 86400 }
  github: { type: http, behavior: domain, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geosite/github.mrs, interval: 86400 }
  geolocation-cn: { type: http, behavior: domain, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geosite/cn.mrs, interval: 86400 }
  cn-ip: { type: http, behavior: ipcidr, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geoip/cn.mrs, interval: 86400 }
rules:
  - RULE-SET,category-ads-all,广告拦截
  - RULE-SET,openai,AI 服务
  - RULE-SET,anthropic,AI 服务
  - RULE-SET,youtube,YouTube
  - RULE-SET,github,GitHub
  - RULE-SET,geolocation-cn,国内直连
  - RULE-SET,cn-ip,国内直连,no-resolve
  - MATCH,漏网之鱼
"""

    saved_yaml_2 = demo_nodes_yaml + """proxy-groups:
  - { name: 节点选择, type: select, proxies: [自动选择, "香港 01 · IPLC", "新加坡 01 · 狮城"] }
  - { name: 自动选择, type: url-test, proxies: ["香港 01 · IPLC", "香港 02 · BGP", "新加坡 01 · 狮城"], url: https://www.gstatic.com/generate_204, interval: 300 }
  - { name: 国内直连, type: select, proxies: [DIRECT, 节点选择] }
  - { name: 漏网之鱼, type: select, proxies: [节点选择, DIRECT] }
rule-providers:
  geolocation-cn: { type: http, behavior: domain, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geosite/cn.mrs, interval: 86400 }
  cn-ip: { type: http, behavior: ipcidr, format: mrs, url: https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@meta/geo/geoip/cn.mrs, interval: 86400 }
rules:
  - RULE-SET,geolocation-cn,国内直连
  - RULE-SET,cn-ip,国内直连,no-resolve
  - MATCH,漏网之鱼
"""

    tpl_1 = json.dumps({
        "preset": "standard",
        "enabled_modules": ["select", "auto", "final", "ad", "private", "cn", "ai", "google", "youtube", "github", "telegram", "netflix"],
        "disabled_rule_sets": [],
        "custom_rule_sets": [],
        "custom_rule_lines": ["DOMAIN-SUFFIX,claude.ai,AI 服务", "DOMAIN-SUFFIX,cursor.sh,AI 服务"],
        "rule_order": [],
        "policy_groups": [
            {
                "id": "custom-work",
                "name": "办公专线",
                "kind": "select",
                "members": ["自动选择", "DIRECT"],
                "include_all_nodes": True,
                "builtin": False,
                "disabled": False,
                "removed": False
            }
        ],
        "group_order": [],
        "rule_set_targets": {},
        "custom_groups": [],
        "chains": [],
        "test_url": "https://www.gstatic.com/generate_204",
        "test_interval": 300,
        "test_tolerance": 50,
        "mixed_port": 7897,
        "allow_lan": False
    }, ensure_ascii=False)

    tpl_2 = json.dumps({
        "preset": "minimal",
        "enabled_modules": ["select", "auto", "final", "ad", "private", "cn"],
        "disabled_rule_sets": [],
        "custom_rule_sets": [],
        "custom_rule_lines": [],
        "rule_order": [],
        "policy_groups": [],
        "group_order": [],
        "rule_set_targets": {},
        "custom_groups": [],
        "chains": [],
        "test_url": "https://www.gstatic.com/generate_204",
        "test_interval": 300,
        "test_tolerance": 50,
        "mixed_port": 7897,
        "allow_lan": False
    }, ensure_ascii=False)

    now = int(time.time())
    with sqlite3.connect(db_path) as conn:
        conn.executescript(schema)
        conn.execute("PRAGMA user_version=3")
        conn.execute(
            "INSERT INTO subscriptions(name,kind,url,payload,node_count,traffic_upload,traffic_download,traffic_total,expire_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            ("CloudEdge Pro (演示)", "url", "https://sub.example.com/api/v1/client/subscribe?token=demo", demo_nodes_yaml, 10,
             18 * 1024**3, 94 * 1024**3, 500 * 1024**3, now + 45 * 86400, now - 3600, now - 600),
        )
        c1 = "a1b2c3d4e5f60718293a4b5c6d7e8f01"
        c2 = "f0e1d2c3b4a5968778695a4b3c2d1e02"
        conn.execute(
            "INSERT INTO saved_configs(id,name,yaml,source_json,created_at,updated_at) VALUES(?,?,?,?,?,?)",
            (c1, "日常主力 · AI & 流媒体", saved_yaml_1, json.dumps({"routing": json.loads(tpl_1)}), now - 1800, now - 120),
        )
        conn.execute(
            "INSERT INTO saved_configs(id,name,yaml,source_json,created_at,updated_at) VALUES(?,?,?,?,?,?)",
            (c2, "极简低延迟 · 办公直连", saved_yaml_2, None, now - 7200, now - 900),
        )
        conn.execute(
            "INSERT INTO templates(name,builtin,config_json,created_at,updated_at) VALUES(?,0,?,?,?)",
            ("AI 与流媒体全能模板", tpl_1, now - 3600, now - 300),
        )
        conn.execute(
            "INSERT INTO templates(name,builtin,config_json,created_at,updated_at) VALUES(?,0,?,?,?)",
            ("极简办公直连", tpl_2, now - 7200, now - 1800),
        )
        conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('server_bind_lan','true')")
        conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('preset','\"standard\"')")
        conn.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES('enabled_modules',?)",
            (json.dumps(["select", "auto", "final", "ad", "private", "cn", "ai", "google", "youtube", "github", "telegram", "netflix"]),),
        )
        conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('theme_mode',?)", (theme,))
        conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('close_to_tray','false')")


def kill_swiftsub():
    subprocess.run(["taskkill", "/F", "/IM", "swiftsub.exe"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.4)


def capture_one(args, out_rel):
    fixture = pathlib.Path(tempfile.mkdtemp(prefix="swiftsub-shot-"))
    app_dir = fixture / "SwiftSub"
    app_dir.mkdir()
    seed_demo_db(app_dir, theme="dark")
    env = dict(os.environ, SWIFTSUB_DATA_DIR=str(app_dir), APPDATA=str(fixture))
    proc = subprocess.Popen([str(BINARY), *args], env=env)
    try:
        time.sleep(1.8)
        res = subprocess.run(
            [
                "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass",
                "-File", str(CAPTURE_PS1),
                "-Out", out_rel,
                "-ProcessId", str(proc.pid),
            ],
            cwd=str(ROOT), check=True, capture_output=True, text=True
        )
        print(res.stdout.strip())
        out_path = ROOT / out_rel
        data = out_path.read_bytes()
        if len(data) < 25_000:
            raise RuntimeError(
                f"{out_rel} looks blank ({len(data)} bytes). "
                "Please run this script in a normal unlocked Windows desktop session."
            )
        return hashlib.sha256(data).hexdigest()
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=3)
        time.sleep(0.3)
        shutil.rmtree(fixture, ignore_errors=True)


def main():
    if not BINARY.is_file():
        print(f"Release binary not found: {BINARY}", file=sys.stderr)
        print("Build it first: .\\scripts\\cargo-msvc.bat build --release --locked -p swiftsub-app", file=sys.stderr)
        sys.exit(1)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    jobs = [
        (["--page=generator"], r"docs\assets\preview-generator.png"),
        (["--page=configs"], r"docs\assets\preview-configs.png"),
        (["--open-template=AI 服务"], r"docs\assets\preview-templates.png"),
        (["--settings-tab=data"], r"docs\assets\preview-settings.png"),
    ]
    hashes = {}
    for args, out_rel in jobs:
        digest = capture_one(args, out_rel)
        if digest in hashes.values():
            raise RuntimeError(f"Duplicate screenshot detected for {out_rel}!")
        hashes[out_rel] = digest
    print("All 4 distinct preview screenshots captured successfully into docs/assets/!")


if __name__ == "__main__":
    main()
