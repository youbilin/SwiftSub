# 分流配置模板

核对与验证日期：2026-11-14。

模板保存完整的分流配置：启用的内置策略组、自定义策略组及其类型和成员顺序、筛选代理组、链式代理、规则集开关、多来源规则绑定、内置规则目标覆盖、自定义规则、规则镜像及生成参数。旧模板缺少新增字段时按空值加载。

## 配置流程

1. 在“配置生成”点击“规则集”或“存为模板”，打开“分流配置模板”。
2. 在“策略组”启用内置组，或输入名称新建自定义组。可选手动选择（select）、自动测速（url-test）、故障转移（fallback）、负载均衡（load-balance）。
3. 勾选组成员，成员按勾选顺序保存；取消后重新勾选可调整顺序。“全部节点”在生成时加入当前启用的节点。测速类组使用设置中的测速 URL 与周期。按地区、关键词或订阅筛选节点可先创建筛选代理组，再将它作为策略组成员。
4. 到“规则集”按来源勾选规则，支持内置 MetaCubeX、MetaCubeX 扩展、blackmatrix7、Loyalsoldier、ACL4SSR 和手动链接同时使用。各来源的全选和清空独立生效。
5. 点击每条规则右侧蓝色目标，选择内置组、自定义组或 DIRECT / REJECT / REJECT-DROP。选择未启用的内置组会同时启用该组；选择外部规则时也会启用对应默认服务组。不会悄悄改绑到“节点选择”。
6. 在“自定义规则”填写 Mihomo 规则，例如 `DOMAIN-SUFFIX,example.com,流媒体专线`。目标必须存在。
7. “仅应用”更新当前配置；填写模板名称后“保存并应用”可在模板页再次恢复。取消会丢弃本次编辑。

例如可把内置 Netflix 域名规则、blackmatrix7 的 Netflix 规则和 ACL4SSR 的 Netflix 规则都绑定到“流媒体专线”，把 Netflix IP 规则绑定到“测速专线”。规则按生成的 YAML 顺序匹配；重叠的规则以先命中的为准。希望使用某个来源替代内置规则时，应关闭对应内置规则。

## 校验与错误处理

应用或保存前先在独立配置中完整生成和校验，再更新当前配置。组名不得为空、包含逗号或换行，或与现有组、节点、保留策略重名。空成员、未知目标、自引用和间接循环均报错；被规则或其他组引用的自定义组不能直接删除。无效模板不会部分覆盖当前配置。

新策略组的成员可选择其他组、内置策略与全部节点；精确的节点筛选沿用主页面的筛选代理组。链式代理沿用专门的链式配置入口。

## 参考 SubBoost 的行为

核对了 SubBoost 官方仓库的模板数据结构、模板创建与恢复、策略组生成和规则目标选择流程。其模板将启用组、自定义组、策略组高级配置、外部规则集和目标绑定一起保存。SwiftSub 依此补齐上述交互与数据流，采用独立 Rust / GPUI 实现；不宣称覆盖 SubBoost 的全部高级功能。

官方源码入口：

- [模板数据结构](https://github.com/SubBoost/subboost/blob/main/packages/core/src/types/template-config.ts)
- [模板创建与校验](https://github.com/SubBoost/subboost/blob/main/packages/core/src/templates/config-template.ts)
- [模板应用](https://github.com/SubBoost/subboost/blob/main/packages/ui/src/store/config-store/actions/template-actions.ts)
- [策略组生成](https://github.com/SubBoost/subboost/blob/main/packages/core/src/generator/proxy-groups.ts)

## 本次验证

- `scripts\cargo-msvc.bat test --workspace`：51 项测试全部通过，覆盖旧模板读取、模板序列化恢复、所有组类型、成员顺序、内置和外部目标、未知目标拒绝、名称冲突、空成员、自引用、间接循环及有效前向引用。
- 独立数据目录中的实际窗口操作：创建手动组与测速组；修改内置 Netflix 域名和 IP 目标；选择 blackmatrix7、ACL4SSR 规则并绑定自定义组；保存、自定义规则生成、取消编辑、重启恢复、切换精简预设后重新应用保存模板。重新应用的 YAML 与原始保存结果完全一致。
- Mihomo v1.19.34 `-t`：四种策略组、四个外部规则来源与现代 REALITY / XHTTP 节点混合配置校验成功。

配置校验验证语法和引用关系；未使用示例节点测试真实代理连接。
