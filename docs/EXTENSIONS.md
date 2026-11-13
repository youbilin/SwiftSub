# 扩展配置

生成页 → 编辑分流 → 高级配置。编辑 JSON 后先点击「校验并更新草稿」，再点击「仅应用」或「保存并应用」。未校验的文本会自动保存为草稿，但不会直接发布到订阅。

```json
{
  "dns": {
    "ipv6": false,
    "fake_ip_range": "198.18.0.1/16",
    "domestic": ["https://dns.alidns.com/dns-query"],
    "foreign": ["https://cloudflare-dns.com/dns-query"]
  },
  "listeners": [
    {"name": "private-socks", "port": 19090, "proxy": "节点选择", "kind": "socks", "listen": "127.0.0.1", "udp": true}
  ],
  "chains": [],
  "custom_groups": [],
  "extra_yaml": "log-level: warning\n"
}
```

监听 `kind` 可选 mixed / http / socks；端口不能与主 mixed-port 或其他监听重复。`proxy` 必须是存在的策略组、节点、DIRECT 或 REJECT。默认仅回环监听；如设置其他 IP，应使用目标机器存在的地址。

三跳链示例（将节点名替换为已导入节点，也可使用诊断报告中的 source_key）：

```json
{"name":"三跳链","hops":["香港 01","日本 01","美国 01"]}
```

把该对象加入 `chains` 数组。每跳必须唯一、存在且启用，支持 2–16 跳，生成组指向最终落地节点。旧版 entry_node / landing_node 双跳格式仍兼容。源订阅更新后节点缺失时停止输出并显示错误，需要重新选择链路。

扩展 YAML 必须为顶层映射，不能覆盖 proxies / proxy-groups / rules / rule-providers / listeners / mixed-port / allow-lan。其他字段由 Mihomo 解释；高级字段如 TUN 仍需按实际权限和客户端版本验证。本工具的结构校验不替代客户端 `-t` 与连接测试。

多配置使用共享订阅缓存与节点编辑，但规则、DNS、监听、链路、订阅成员与文件名独立保存。包含全部订阅模式会包含以后新增来源；逐项选择模式保留固定来源 ID。删除源订阅也会影响所有引用它的配置。没有可用输出时该配置地址返回 503；删除配置后其 Token 返回 404。

二维码的节点内容是完整 `proxies` YAML，可扫描后复制回 SwiftSub 导入。其他客户端是否支持直接扫描 YAML 由对应客户端决定。订阅 URL 维持 127.0.0.1，仅供同一台电脑上的客户端使用。

测试生成示例：

```bat
scripts\cargo-msvc.bat run -p swiftsub-core --example extension_profile -- extensions.yaml
mihomo.exe -t -d <含 GeoSite.dat 的目录> -f extensions.yaml
```
