# SwiftSub

[![Validate Windows](https://github.com/youbilin/SwiftSub/actions/workflows/validate.yml/badge.svg)](https://github.com/youbilin/SwiftSub/actions/workflows/validate.yml)
[![License](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)

**SwiftSub** 是一款基于 **Rust + GPUI** 构建的本地 **Clash / Mihomo** 订阅转换、规则编排与局域网分发桌面工作台。

它用于将机场订阅链接、单/多行节点分享链接、Base64 订阅内容或已有 Clash YAML 转换为结构规范、经过校验的 Mihomo 配置文件，并通过内置 HTTP 服务以固定地址提供给本机与局域网内的客户端持续拉取更新。所有配置、节点、缓存和订阅数据均仅存储于本机，不依赖任何云端后端。

---

## 核心特性

### 1. 多协议节点解析与 Mihomo 精确导出
- **输入格式**：HTTP/HTTPS 远程订阅（支持 `Subscription-Userinfo`、`Profile-Update-Interval`、`ETag` / `Last-Modified` 条件刷新）、单/多行分享链接、Base64 编码节点列表、Clash / Mihomo YAML 配置文件。
- **节点去重与覆写**：按语义指纹自动去重，支持单节点完整 YAML 编辑、改名（自动同步所有策略组、规则、链式与监听器引用）、启停开关及拖拽排序。
- **支持协议与最新参数**：

| 协议 | 支持的分享格式与关键参数 |
|---|---|
| **VLESS** | `REALITY`（`pbk`/`publicKey`、`sid`、`support-x25519mlkem768` 默认开启、非空 `pqv` 显式报错防静默降级）、`flow`、`encryption`、`packet-encoding`、`XHTTP / SplitHTTP`（含 `mode`、`extra` / `xmux` 映射与回退）、`WS`（含 `?ed=` Early Data 自动拆分）、`HTTPUpgrade`、`gRPC`、`H2`、`HTTP`、`ECH`、`TFO`、IPv6 |
| **Trojan** | 正确输出 Mihomo 要求的 `sni` 字段、含冒号密码完整保留、Trojan REALITY、`WS / gRPC / HTTP / H2 / XHTTP`、`alpn`、`client-fingerprint`、`ECH`、`TFO`、IPv6 |
| **Hysteria2** | `hy2://` 与 `hysteria2://`、`sni`、`obfs`（自动忽略无效 `obfs=none`）、`obfs-password`、`up/down` 带宽自动规范为 `Mbps`、`ports`/`mport`、`hop-interval`、`cwnd`、`pinSHA256`/`fingerprint`、`alpn`、`ECH`、`TFO`、IPv6 |
| **TUIC** | TUIC v5（`uuid:password`）与 v4（`token`）、`congestion-controller`、`udp-relay-mode`、`sni`、`disable-sni`、`reduce-rtt`、`heartbeat-interval`、`request-timeout`、`alpn`、`client-fingerprint`、`ECH`、`TFO`、IPv6 |
| **Shadowsocks / SSR** | SIP002 / Legacy Base64、`obfs`、`v2ray-plugin`（布尔 `mux`、`skip-cert-verify`、`fingerprint`）、`shadow-tls`、`restls`、`udp-over-tcp`（`uot`）、`TFO`；SSR 完整协议/混淆参数 |
| **VMess** | 标准 V2RayN JSON 与 Shadowrocket URI 格式、`WS`（含 `?ed=`）、`gRPC`、`H2`、`HTTP`、`XHTTP`、`packet-encoding`、`ECH`、`TFO`、IPv6 |
| **扩展与基础协议** | `AnyTLS`、`Snell`（v1–v3）、`WireGuard`（`wg://`）、`SSH`、`HTTP / HTTPS`、`SOCKS4 / SOCKS5`（含 Base64 凭据与 TLS/SNI）；Clash YAML 导入原样保留 `mieru`、`juicity`、`masque`、`direct`、`dns`、`reject` 及未知扩展字段 |

### 2. 可视化分流、链式代理与独立配置管理
- **三页精简架构**：顶层仅保留 **「生成 / 配置 / 设置」**，左侧节点列表支持宽度拖拽与搜索，右侧管理策略组与链式代理。
- **策略组与规则目录**：
  - 内置精简 / 标准 / 完整三套预设与自定义筛选代理组（按地区、来源、关键词包含/排除）。
  - 集成 MetaCubeX、blackmatrix7、Loyalsoldier、ACL4SSR 规则目录，支持自定义 `.mrs`/`.yaml`/`.txt` 规则集链接、手写规则，以及本地规则正文预览与覆写。
  - 保存前执行引用完整性与策略组循环依赖校验。
- **链式代理（Dialer Proxy）**：下拉选择入口与出口节点即可生成链式节点，支持一键互换两端、独立启停与混排。
- **多配置库与统一编辑**：每份保存的配置对应独立的 YAML 文件与固定订阅 ID，支持导入已有 Clash YAML 可视化编辑、完整 YAML 源码编辑、外部文件改动冲突保护。
- **完整 DNS 与测速配置**：表单与 YAML 双模式编辑 Mihomo `dns` 与 `hosts`（保留未列出扩展字段），支持全局与单组测速参数。

### 3. 本机与局域网订阅服务
- **独立持久化地址**：每份配置拥有固定 URL 与独立文件，保存即发布，支持 `ETag` / `If-None-Match`（304）、`HEAD` 与拉取状态追踪：
  ```text
  本机地址：  http://127.0.0.1:<端口>/sub/<配置ID>/<配置ID>.yaml
  局域网地址：http://<物理网卡IPv4>:<端口>/sub/<配置ID>/<配置ID>.yaml
  配置文件：  %APPDATA%\SwiftSub\configs\<配置ID>.yaml
  ```
- **局域网导入支持**：
  - 支持在「配置」页或「设置 → 应用与更新」一键开启局域网监听（`0.0.0.0`），自动避开 Clash 常用端口冲突（默认回退至 `17899–17920`）。
  - Windows 原生通过 `GetAdaptersAddresses` 枚举在线网卡并智能排序，**自动排除 Mihomo / Clash TUN 虚拟网段（`198.18.0.0/15`）、回环、APIPA 与 VMware / Hyper-V 虚拟网卡**，优先命中真实物理以太网 / Wi-Fi，同时支持手动切换网卡（含 Tailscale 等组网网卡）。
- **本机一键导入 Clash Verge**：调用 `clash://install-config`（兼容 `clash-verge://`）导入并观测客户端拉取结果。
- **托盘保活与版本更新检查**：支持关闭窗口最小化到 Windows 系统托盘持续提供订阅服务；内置 GitHub Releases 更新检查接口，可一键检查新版本并跳转下载。

---

## 架构设计

工作区采用 5 个解耦 Crate 分层组织，核心逻辑完全独立于 UI，便于单元测试与复用：

```text
crates/
├── swiftsub-core/     节点链接/YAML 解析、规范化、策略组、规则校验、MRS 解码、DNS 与配置生成
├── swiftsub-fetch/    HTTP 订阅抓取、代理模式、Userinfo 解析、GitHub Releases 更新检查接口
├── swiftsub-store/    SQLite 持久化、事务迁移、独立配置文件管理与覆盖保护
├── swiftsub-server/   内置本地/局域网 HTTP 订阅服务（ETag、条件请求、Windows 网卡智能探测）
└── swiftsub-app/      GPUI 0.9.1 桌面端界面、状态管理与 Windows 托盘/URI 集成
```

---

## 安装与构建

### 下载预编译版本
前往 [Releases](https://github.com/youbilin/SwiftSub/releases) 下载最新的 Windows x64 可执行文件 `swiftsub-vX.Y.Z-windows-x64.exe`，双击即可运行（便携单文件，无需安装）。

### 从源码构建

**环境要求**：
- Windows 10 / 11 x64
- Rust `1.99.0`（已在 [rust-toolchain.toml](rust-toolchain.toml) 锁定）
- Visual Studio Build Tools（Desktop development with C++）与 Windows SDK

**常用命令**：

```powershell
# 运行开发构建
.\scripts\cargo-msvc.bat build -p swiftsub-app

# 全工作区格式检查、严格 Clippy 与回归测试
.\scripts\cargo-msvc.bat fmt --all -- --check
.\scripts\cargo-msvc.bat clippy --workspace --all-targets --locked -- -D warnings
.\scripts\cargo-msvc.bat test --workspace --locked

# 构建 Release 产物（启用 thin LTO 与符号剥离）
.\scripts\cargo-msvc.bat build --release --locked -p swiftsub-app
# 输出路径: target\release\swiftsub.exe
```

---

## 快速上手

1. **导入来源**：在「生成」页左侧点击「添加节点 / 订阅」，粘贴订阅链接、多行节点链接或导入已有 Clash YAML。
2. **编排分流**：在右侧展开或新建策略组、绑定规则集/手写规则，或在「链式」标签页创建中转落地组合。
3. **保存与分发**：点击顶部「保存」生成独立配置与固定订阅地址；
   - 本机客户端：点击「导入到 Clash」或复制本机订阅地址；
   - 手机 / 平板 / 软路由等局域网设备：在「配置」页或「设置 → 应用与更新」开启 **「局域网导入」**，复制局域网订阅地址（若无法访问请在 Windows 防火墙放行 SwiftSub）。
4. **模板复用与更新**：在「设置 → 模板」可将当前分流结构存为自定义模板并随时编辑/覆盖；在「设置 → 应用与更新」可开启托盘保活及一键检查 GitHub 新版本。

---

## 隐私与安全说明

- **纯本地运行**：SQLite 数据库、规则缓存与生成的 YAML 配置文件默认存放于 `%APPDATA%\SwiftSub\`，不包含任何账号系统、遥测或第三方统计。
- **网络访问范围**：仅在用户主动刷新远程订阅、更新规则源或点击「检查更新」（访问 GitHub Releases API）时发起对外请求；本地订阅服务默认仅绑定 `127.0.0.1`，仅在用户显式开启「局域网导入」后监听 `0.0.0.0`。
- **免责声明**：本项目仅作为本地配置文件编排与订阅格式转换工具，不提供任何代理节点或网络转发服务。

## 致谢与许可

- 部分功能思路参考 [SubBoost](https://github.com/SubBoost/subboost)（AGPL-3.0），本项目为完全独立实现，不包含其源码或资源。
- 本项目以 **Apache License 2.0** 开源，详见 [LICENSE](LICENSE)。
