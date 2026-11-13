# 链接参数适配

核对日期：2026-11-14。Xray-core 最新正式版为 v26.11.9（2026-11-09 发布）；输出目标为 Mihomo YAML，配置语法校验使用 v1.19.34。

验证记录：全工作区 51 项测试通过；实际生成同时含四个目录来源、26 个 rule-providers 以及默认 MLKEM REALITY + XHTTP、显式关闭 MLKEM 的 REALITY、RAW HTTP 伪装和 HTTPUpgrade 共 4 个节点的配置，通过 `mihomo v1.19.34 -t`。桌面界面实测来源全选、单独清空、展开和应用，确认 45 条外部规则集从三个来源共同持久化并写入实际生成配置。

## REALITY

VLESS 分享链接的 `pbk`（也接受 `publicKey` / `public-key`）、`sid`、`sni`、`fp` 分别映射为 `reality-opts.public-key`、`reality-opts.short-id`、`servername`、`client-fingerprint`。即使只通过公钥别名识别出 REALITY，也会启用 TLS。

`support-x25519mlkem768` 写入 `reality-opts`，类型为布尔值。接受 true/false、1/0、yes/no、on/off（大小写不敏感），以及 `support_x25519mlkem768`、`supportX25519MLKEM768` 别名。未指定时默认 true，显式 false 保留，非法值报告错误。这个默认值用于新服务端；旧服务端可在链接中显式关闭。

`pqv` / `mldsa65Verify` 是 ML-DSA-65 公钥验证参数，与 X25519-MLKEM768 密钥交换开关不同。当前 Mihomo REALITY 配置没有对应验证字段，非空值会使该链接导入失败并显示原因，空值按未配置处理。

协议适配不等于实际握手保证。Mihomo 官方文档对 Xray v26.7.11+ REALITY 兼容性有明确限制；实际连接仍取决于服务端版本、指纹和客户端实现。当前已增加下述受控回环实际传输测试；公网服务端和其他版本仍需分别验证。

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

## 2026-11-14 实际回环传输复核

使用官方 Xray v26.11.9 Windows amd64、Mihomo v1.19.34 和本地 Go 1.24.7 TLS 1.3 目标，客户端配置由实际链接解析器及生成器生成。请求经 Mihomo HTTP 代理、Xray VLESS 到本地 HTTP 服务，检查完整响应内容。

| 用例 | 结果 |
| --- | --- |
| RAW + REALITY，参数省略（默认 true） | 实际传输通过 |
| XHTTP packet-up，无 REALITY | 实际传输通过 |
| RAW + REALITY，support-x25519mlkem768=true | 实际传输通过 |
| XHTTP packet-up + REALITY，support-x25519mlkem768=true | 实际传输通过 |
| RAW + REALITY，support-x25519mlkem768=false | 失败，Mihomo 返回 HTTP 502 |

这组测试说明 true 适配本次测试的新版服务端；false 仍须原样保留供旧环境使用，但不能保证与新版服务端兼容。伪装目标会影响测试：Python/OpenSSL 目标在初次试验中握手失败，换用 Go TLS 目标后上述三个用例通过。不得据此承诺所有指纹、模式或公网目标均可用。

复现：先构建 `cargo build -p swiftsub-core --example validation_profile`，再用 `go build -o tls-target.exe scripts/fixtures/tls-target.go` 构建目标。运行 `python scripts/validate-loopback.py --xray <xray.exe> --mihomo <mihomo.exe> --data-dir <含 GeoSite.dat 的目录> --tls-target <tls-target.exe> --openssl <openssl.exe>`。工具仅在回环地址启动测试服务，结束后清理拥有的进程；配置与日志保存在它输出的临时目录。任一传输失败时脚本返回非零；显式 false 用例对该新版服务端预期失败。生成的测试配置和密钥只用于本次回环服务。

## 导入与输出报告

生成页「诊断」提供逐节点解析状态；链接按原始行号、YAML 按 proxies 项序号标记成功、需检查或失败。生成报告包含节点来源、组成员、最终规则及配置正文 SHA-256。YAML 顶部记录同一个摘要，便于核对导出的报告与配置。报告不会保存原始节点链接或节点认证字段。
