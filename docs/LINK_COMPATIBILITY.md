# 链接参数适配

核对日期：2026-11-14。Xray-core 最新正式版为 v26.11.9（2026-11-09 发布）；输出目标为 Mihomo YAML，配置语法校验使用 v1.19.34。

验证记录：全工作区 51 项测试通过；实际生成同时含四个目录来源、26 个 rule-providers 以及默认 MLKEM REALITY + XHTTP、显式关闭 MLKEM 的 REALITY、RAW HTTP 伪装和 HTTPUpgrade 共 4 个节点的配置，通过 `mihomo v1.19.34 -t`。桌面界面实测来源全选、单独清空、展开和应用，确认 45 条外部规则集从三个来源共同持久化并写入实际生成配置。

## REALITY

VLESS 分享链接的 `pbk`（也接受 `publicKey` / `public-key`）、`sid`、`sni`、`fp` 分别映射为 `reality-opts.public-key`、`reality-opts.short-id`、`servername`、`client-fingerprint`。即使只通过公钥别名识别出 REALITY，也会启用 TLS。

`support-x25519mlkem768` 写入 `reality-opts`，类型为布尔值。接受 true/false、1/0、yes/no、on/off（大小写不敏感），以及 `support_x25519mlkem768`、`supportX25519MLKEM768` 别名。未指定时默认 true，显式 false 保留，非法值报告错误。这个默认值用于新服务端；旧服务端可在链接中显式关闭。

`pqv` / `mldsa65Verify` 是 ML-DSA-65 公钥验证参数，与 X25519-MLKEM768 密钥交换开关不同。当前 Mihomo REALITY 配置没有对应验证字段，非空值会使该链接导入失败并显示原因，空值按未配置处理。

协议适配不等于实际握手保证。Mihomo 官方文档对 Xray v26.7.11+ REALITY 兼容性有明确限制；实际连接仍取决于服务端版本、指纹和客户端实现。本项目的配置校验不包含真实服务端连接测试。

## VLESS 与 XHTTP

| 分享链接 | Mihomo 输出 |
| --- | --- |
| `type=raw` / `tcp` | `network: tcp` |
| `type=splithttp` / `xhttp` | `network: xhttp` |
| `type=websocket` / `ws` | `network: ws` |
| `type=httpupgrade` | `network: ws` 和 `ws-opts.v2ray-http-upgrade: true` |
| `headerType=http`（RAW/TCP） | HTTP 传输 |
| `mode` / `xhttp-mode` | `xhttp-opts.mode` |
| `extra` JSON | 已支持的 XHTTP 扩展逐字段转换 |
| `extra.xmux` | `xhttp-opts.reuse-settings` |
| `extra.sessionIDPlacement/Key/Table/Length` | `session-placement/key/table/length` |
| `x_padding_bytes` | `xhttp-opts.x-padding-bytes`（extra 内对应配置优先） |
| 非 none 的 `encryption` | 原样保留为 `encryption` |

XHTTP 支持 auto、packet-up、stream-up、stream-one。依照 Xray 的 `SplitHTTPConfig.Build`，外层 host/path/mode 优先于 extra 中的同名项。扩展中的数值范围转换为 Mihomo 接受的字符串；布尔值、headers 和保活周期保持对应类型。尚未转换的 extra 字段（例如 downloadSettings）会给出包含字段名的提示，需要手动检查，不能视为完整支持。

XDRIVE、MASQUE 等目标客户端未支持的 VLESS 传输会报告错误。其余协议的原有解析行为保持兼容。

完整的策略组与规则目标保存、恢复及验证记录见 [分流模板说明](ROUTING_TEMPLATES.md)。

## 规则来源

内置 MetaCubeX 规则与 MetaCubeX 扩展、blackmatrix7、Loyalsoldier、ACL4SSR 按分类显示。选择保存在原有模板规则绑定列表中，以来源前缀区分 ID；不同来源不会覆盖。清空或全选仅影响当前来源，搜索期间仅影响该来源的匹配项。

镜像是下载地址选择，不是规则仓库单选。更改镜像后，已选目录规则集会重新解析 URL，手动添加的规则链接保持原地址。

## 官方参考

- [Xray-core v26.11.9](https://github.com/XTLS/Xray-core/releases/tag/v26.11.9)
- [Xray REALITY 文档](https://xtls.github.io/config/transport.html#realityobject)
- [Xray 分享链接讨论](https://github.com/XTLS/Xray-core/discussions/716)
- [Xray XHTTP 说明](https://github.com/XTLS/Xray-core/discussions/4113)
- [Xray 传输和 XHTTP 配置构建代码](https://github.com/XTLS/Xray-core/blob/main/infra/conf/transport_method.go)
- [Mihomo TLS / REALITY 文档](https://wiki.metacubex.one/config/proxies/tls/)
- [Mihomo VLESS 文档](https://wiki.metacubex.one/config/proxies/vless/)
- [Mihomo REALITY 字段定义](https://github.com/MetaCubeX/mihomo/blob/Meta/adapter/outbound/reality.go)
- [Mihomo XHTTP 字段定义](https://github.com/MetaCubeX/mihomo/blob/Meta/adapter/outbound/vless.go)
