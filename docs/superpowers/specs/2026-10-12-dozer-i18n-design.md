# Dozer 多语言支持规格

> 状态：Draft，供实现与评审；本次只新增规格，不表示功能已经实现。
> 日期：2026-10-12。实现基线：当前工作区代码（包含尚未提交的修改）。
> 多语言指面向人的界面语言；首批支持 `en` 与 `zh-CN`。

## 1. 依据与目标

本规格以 [V2 架构分析](../../dozer-v2/dozer-v2架构分析.md) §19 和 [V2 静态结构图](../../dozer-v2/dozer-v2静态结构图.md) B/C/D/V 为设计输入：采用扁平 JSON 字符串表、命名空间 key、声明式标题 fallback、daemon 持久化语言偏好以及不同 Surface 的语言传递边界。

两份文档整体属于方向分析，图中的 `dozer-host`、`dozer-protocol`、Plugin Runtime 与完整 Contribution Registry 不能视为现有实现。本规格保留其 i18n 语义，按当前 `dozer-app` / `dozer-model` / `dozer-services` / `dozer-client` / `dozerd` 的依赖边界落地，不以完整 V2 插件化或 Workflow Kernel 为前置条件。

用户应能在设置中选择 English 或简体中文，使 Dozer 自有界面在不重启应用、不丢失工作状态的情况下切换，并在下次启动时恢复选择。独立面板 + Host 的组合使用同一机制，不依赖另一个面板提供翻译能力。

## 2. 当前实现与缺口

以下为已核对的代码事实；后文新增类型、模块与字段均为提案。

| 现有位置 | 现状 | 本规格要求 |
|---|---|---|
| `crates/dozer-app/src/product.rs`、`panel_registry.rs` | 产品组合根提供 `PanelDescriptor`，标题是静态字符串；清单通过 `OnceLock` 安装 | 注册稳定 key/fallback，渲染时解析，不能启动时翻译后永久缓存 |
| `crates/dozer-app/src/extensions/settings.rs` | 设置包含 Theme/Git/Advanced/Apps；无界面语言选择 | 新增语言页，复用独立 settings overlay |
| `crates/dozer-app/src/menu_spec.rs`、`chrome/native_menu.rs` | 同一菜单内容供原生及 iced 后端消费 | 在内容组装层本地化，保持 action 与快捷键不变 |
| `crates/dozer-app/src/extensions/todo/protocol.rs`、`web/todo-content/src/protocol.ts` | 已有 `protocol_version`、`revision`、ready、推送去重和重建重发机制 | 扩展 locale 元数据并纳入去重，保留原有时序语义 |
| `crates/dozer-app/src/extensions/codehealth/protocol.rs` | 部分 payload 在 Rust 侧已拼接中文，例如语言统计详情 | 改为语义数据或 key + 参数，不能只翻译 JSX |
| `crates/dozer-app/src/app/update.rs` | 主题切换会触发部分预览 WebView 重载 | 语言切换使用状态推送，不能照搬重载流程 |
| `crates/dozer-services/src/lib.rs` | 面板通过服务 trait 取数；`ServiceError` 只有类别和 message | 新增语言设置服务；错误展示与诊断字符串分离 |
| `crates/dozer-model/src/`、`dozer-core/src/protocol.rs` | 领域 DTO 与 UDS 请求/应答已分层 | DTO 在 model，传输在 core，面板不得越过 services 直连 client |
| `crates/dozer-app/src/extensions/toast.rs` | Toast 保存文本，带稳定 id、去重 key、到期时间 | 迁移为可重渲染文案，切换不能重复通知或续期 |
| `crates/dozer-app/Cargo.toml` | bytehost、byteui 已为独立仓库依赖 | 不在 Dozer 重建平台 manifest/parser 或复制组件实现 |
| `crates/dozerd/src/session.rs` | 子进程已有 UTF-8 locale 环境兜底 | 这是终端环境设置，与 UI locale 分离，不随 UI 切换改写 |

当前自有 Rust/TypeScript 源码中未发现统一 i18n/locale 解析层。界面存在中英文硬编码混用；支持中文显示不等于已支持语言选择。

## 3. 范围

必须覆盖 Host 的 Rail、Tab、顶栏、底栏、设置、空态、tooltip、上下文菜单、确认框、Toast、可访问性标签，以及 Dozer 自有面板与内置 WebView 的按钮、状态、错误说明和加载/失败兜底页。

不翻译以下内容：

- 项目名、文件路径、用户输入、Todo 内容、Memory、对话正文、Agent 输出、代码、终端输出、Git 提交消息及数据库数据。
- MCP tool/resource/prompt 标识和 description；描述保持现有契约，新增描述使用英文。存量描述清理不作为本次迁移的一部分。
- 错误码、协议枚举、命令 ID、日志事件标识、审计与历史 Execution 原始内容。
- 浏览器访问的外部网站、用户 HTML/Markdown 的正文，或未声明支持语言切换的第三方应用内部界面。

首批不提供在线翻译、语言包下载市场、RTL、自动翻译用户内容或 Agent 回答语言控制；不新增命令面板。未来命令注册采用同一标题契约。

## 4. 语言偏好及持久化

### 4.1 数据与默认值

新增 `LocalePreference = System | Explicit(en | zh-CN)`，设置显示自称语言名：`English`、`简体中文`，以及可翻译的“跟随系统”。语言选择始终可辨认。

持久化对象包含 `schema_version`、`preference`、`active_locale`、`revision`。其中 `active_locale` 为解析后的支持语言；revision 为 daemon 串行提交且落盘的递增值。偏好属于当前 OS 用户的 Dozer 全局设置，所有项目/窗口共享，不进入项目仓库，不影响其他产品（例如 Digger）的设置。

默认 `System`。系统语言探测由桌面平台适配器提供有序偏好列表，不能仅依赖 GUI/launchd 环境中的 `LANG`。daemon 负责解析及保存结果；无桌面探测值时可读取自己的环境候选，最终兜底 `en`。

匹配规则固定为：

1. 规范化大小写及 `_`/`-` 分隔，环境 locale 移除编码/修饰后缀；非法输入忽略。
2. `en` 及 `en-*` → `en`；`zh`、`zh-CN`、`zh-SG`、`zh-Hans` 及其扩展 → `zh-CN`。
3. 明确 `Hant` 或 `TW/HK/MO` 的中文候选不自动映射简体；优先遵守显式 script，继续查找后续支持语言。
4. 无匹配（包括 `C`/`POSIX`）→ `en`。显式设置不支持的 locale 返回校验错误，不偷偷改成默认值。

跟随系统在应用启动和用户重新选择 System 时重新探测；首版不承诺实时监听 OS 语言变化。仅在 System 模式采用探测值，Explicit 模式不受系统变化影响。

### 4.2 权威与写入

新增 `dozerd/src/locale_settings.rs`，使用 `dozer_core::paths::config_dir()` 下独立 `locale_settings.json`，daemon 为唯一写入者。采用同目录临时文件 + 原子替换；写入失败不推进内存状态或 revision，不向 UI 返回成功。文件缺失按默认初始化；损坏时保留损坏副本、记录诊断，再安全恢复默认，不导致应用无法启动。

服务草案：`get_locale()`、`set_locale(preference, expected_revision, system_candidates)`。读取返回完整快照；写入使用期望 revision，冲突返回最新快照并提示重新选择，不自动覆盖另一个窗口的操作。系统候选只用于 System 解析，无权更改显式偏好。

设置点击后进入保存状态；daemon 提交成功才切换显示。失败保持原语言与选中值，显示旧语言的失败提示。daemon 离线时可继续使用最后确认快照，禁止把未保存选择伪装成成功。

GUI 可在自己的缓存文件保存最后确认快照以支持离线启动；缓存不是第二个配置真相源，不得反向覆盖 daemon。无缓存且 daemon 不可用时，使用系统探测结果，仅为临时显示。

### 4.3 多窗口与重连

同一 GUI 进程通过 App 级消息广播已确认快照，所有窗口及 overlay 共用该状态。首版跨 GUI 进程通过服务读取同步：应用获焦立即刷新，运行期间至多每 2 秒一次；不把尚不存在的通用 Event Bus 当作前提。daemon 重连后重新读取权威快照，替换缓存并刷新 Surface。后续若改为推送订阅，仍需保留重连全量快照。

同一连接周期只应用更新 revision；新连接先建立新基线，避免 daemon 数据恢复后较小 revision 被永久拒绝。

## 5. 分层与资源组织

| 层 | 新增职责 | 不承担 |
|---|---|---|
| `dozer-model/src/i18n.rs` | locale DTO、设置快照、`LocalizedText` 描述（仅数据） | 文件 IO、UI、daemon 连接 |
| 新增 `dozer-i18n` crate | JSON 校验、纯查表、fallback、插值及共享测试向量 | 持久化、iced、网络、插件进程管理 |
| `dozer-services` | `LocaleService` trait，并接入现有服务集合 | 具体传输 |
| `dozer-core` / `dozer-client` | 追加 UDS 请求/应答、客户端及 trait 适配 | 面板翻译逻辑 |
| `dozerd` | 校验、持久化、版本冲突处理 | 拼接人类语言的领域应答 |
| `dozer-app` | 活跃快照、资源装载、渲染、平台探测、Surface 分发 | 为第三方页面注入翻译文本 |

此处将架构分析 §19.5 的“协议契约层解析器”落实为独立纯库，避免让面板为了查表重新依赖 `dozer-core::protocol`，也避免让仅依赖 serde 的 model 承担资源运行时。未来若成立 `dozer-protocol`，复用该契约，不复制解析规则。本次不建立完整 Plugin Protocol。

资源路径提案：

```text
crates/dozer-app/locales/host/{en,zh-CN}.json
crates/dozer-app/locales/panels/<panel-id>/{en,zh-CN}.json   # 仍在 app 内的面板
crates/dozer-panel-usage/locales/{en,zh-CN}.json             # 已独立的面板随其 crate
crates/dozer-app/web/shared/i18n/       # TS 解析器与一致性测试
crates/dozer-i18n/                     # Rust 解析器与一致性测试
```

资源目录跟随资源所有者：位于 `dozer-app` 内的 Host 与面板资源在 `crates/dozer-app/locales/`；已拆为独立 crate 的面板（当前为 `crates/dozer-panel-usage/`）在其自身 `locales/` 下携带资源，locale 由 Host 注入，不得反向依赖 `dozer-app` 的目录，也不得在 app 内维护第二份译文。共享 TS 解析器位于中立目录（`web/shared/i18n/`），各 Web 包静态 import，独立面板的构建可从任意 cwd 定位它，不依赖兄弟工作区运行路径。


同一面板 Rust 与 WebView 从同一组 JSON 取资源，构建时分别打入二进制/前端 bundle，不维护两套译文。资源离线可用，不产生启动网络依赖；具体前端构建入口逐个接线，不假设当前存在共享 npm workspace。byteui 组件通过调用方传入标签，不能持有 Dozer 产品词表。

## 6. 文案契约

### 6.1 key 与 fallback

逻辑身份是 `(owner_id, key)`：Host 使用 `dozer.host`，官方面板使用 `dozer.todo` 等固定 owner；第三方使用安装时验证的应用/插件 ID。示例 key 为 `settings.language.title`、`task.delete.confirm`，不以中文原文作 key，不以展示名当业务身份。

`LocalizedText` 包含 `owner_id`、可选 `key`、必填非空 `fallback` 和具名 `args`。注册标题沿用架构文档字段 `title_key` / `title_fallback`，owner 由注册来源绑定，不能由不可信调用方任意指定其他 owner。

解析顺序：当前 locale 的 key → owner 的 `default_locale` 的 key → 显式 fallback。官方 owner 的 default 为 `en`；第三方遵循其声明，不额外插入英文查找步骤。缺 key、空值、无效模板视为该候选不可用。最终不得返回空白或 panic。诊断记录 owner/key/locale，不记录可能包含用户内容的参数；缺 key 日志去重。

英语资源为官方文案基准，首批全部官方资源要求英中 key 集及占位符集合一致。品牌、SSH、Git、Token 等允许保留原词，但必须明确列入术语表/豁免清单。

### 6.2 插值

采用扁平 JSON 字符串值，只支持 `{name}`，`{{`/`}}` 表示字面花括号。参数名使用 ASCII 字母、数字、下划线且不以数字开头。单次替换，不递归解释参数中的花括号，不支持 HTML、表达式或任意代码。

缺参数或模板非法时依次尝试后续候选；所有模板均不可渲染时显示内置通用错误文案并记录 key。多余参数忽略。资源检查必须发现重复 JSON key、非字符串值、空翻译、错误括号及占位符不一致。

不加入 Fluent/ICU 复数语法。需要数量变化的句子显式定义 `.zero` / `.one` / `.other` 并由调用点选择；首批仅对 en/zh-CN 定义规则，不能据此宣称支持任意语言。

Web 文本通过框架文本节点或 `textContent` 渲染，不能把译文/参数送进 `innerHTML`。确认框里的文件名等动态内容始终按原值安全显示。

### 6.3 格式化与身份

计数、日期、金额首先保持数值/时间戳/币种等语义类型，到展示边界格式化。UI 时间首版统一使用既有时区语义及 `YYYY-MM-DD HH:mm:ss`；整数使用无分组十进制，金额保留明确币种，延续各领域已有精度；后续地区化格式另行扩展。相对时间与“文件数”等单位走词表。切换语言不得改变原始金额、时间值、排序键、CSV/协议输出、数据库枚举或业务判断。

## 7. 原生界面运行时

`App` 持有不可变语言快照和本地显示 generation，将翻译上下文显式传给 view/presenter；不得为适配现有 `&'static str` 签名泄漏字符串。`PanelDescriptor` 保存 key/fallback，标题按当前快照返回借用或拥有的字符串，不重装 OnceLock 清单。

成功切换后必须刷新主窗口、所有已打开 overlay、缓存的 tooltip、Tab/面板标题及菜单。菜单由 `MenuSpec` 统一生成，两个后端共用结果；已打开的原生上下文菜单可关闭，下次以新语言构建。系统自行控制的文件选择器/OS 菜单项允许继续遵循系统语言，Dozer 传入的标题和说明必须翻译。

Toast 和持久错误保存可重渲染描述及独立 raw detail。已显示 Toast 切换语言时更新文本，但不变更 id、去重 key、到期时间，不新增日志。新增通知将稳定事件/错误码与诊断详情写日志，不将本地化文案作为唯一审计依据。旧版历史文本原样保留。

布局不能假设中文字符数等于英文宽度。沿用 UI 系统字体，CJK 自绘文本使用 Advanced shaping；终端/代码字体保持现状。语言切换保留输入、选区、焦点、滚动、拖拽以外的工作状态；如关闭菜单，不触发其业务动作。

## 8. 内置 WebView

在现有推送 envelope 追加可选 `ui_locale` 与 `locale_revision`（后者为 GUI 本地显示 generation，不与 daemon revision 比大小），不修改现有 `payload` 字段意义。每次切换必须同时推进该 Surface 的 envelope `revision`，将 locale 纳入 `last_sent`/desired 比较，业务数据没变也要推送。

```json
{
  "protocol_version": 1,
  "revision": 42,
  "ui_locale": "zh-CN",
  "locale_revision": 3,
  "payload": {}
}
```

示例省略业务内容。前端原子应用 envelope 中的语言和 payload，继续按既有 revision 丢弃旧包，不得让旧业务响应带回旧语言。ready、重新创建、隐藏后恢复都发送最新完整快照；不以“语言事件已发过”为由漏掉新 Surface。

新版前端连接旧 Host 时，缺少语言字段按 `en` 处理；旧前端忽略追加字段可继续运行，但不计为完成双语覆盖。能力不支持与传输故障须区分，不能用失败重试制造无限循环。

前端设置 `document.documentElement.lang`，从 bundle 读取自己的资源。首次 ready 前可渲染无文字 skeleton，避免先中文后英文闪烁。切换不 reload、不重新导航，不修改外部网页的 `navigator.language`。

Code Health 等当前带已格式化文本的 payload，优先追加原始计数、状态 enum 或文案描述，并由前端本地化；迁移期旧字段保留用于兼容。确实由 Rust 渲染的文本必须随语言重新生成，不能永久缓存中文。数据库存量报告和 Agent 生成说明属于内容，保持原文。

## 9. 第三方应用与未来插件

架构分析中的可选 `[i18n]` 契约保留：`default_locale`、`supported_locales`、`strings_dir`，Panel/Page/Tab/Command 使用 `title_key` + 必填 `title_fallback`。无 i18n 声明时直接使用 fallback；声明缺少当前语言时遵循 §6 顺序。

当前 bytehost 应用模型不能等同于完整 V2 Plugin Manifest。涉及安装包 schema、资源路径校验或跨产品公共桌面契约的改动，必须在 bytehost 独立仓库追加兼容能力并发 tag，Dozer 只写消费适配。未经该能力接入的第三方应用仅保证外壳标题显示 fallback，不声称其内部 UI 已本地化。

未来资源装载必须限制在已安装包内，拒绝绝对路径、`..` 和符号链接逃逸，限制资源大小，不允许插件覆盖 Host 的可信确认/权限文案。格式非法的声明在安装校验时报告；运行时资源损坏则使用 fallback，不拖垮 Host。

- Declarative：Host 按 owner 查表。
- WebView：仅向声明接入的应用传 locale，应用自行提供资源；不突破现有 origin/CSP 边界。
- External：启动时传 `DOZER_UI_LOCALE`，不改 `LANG/LC_ALL`。未协商热更新能力的外部窗口在下次启动采用新语言，不强制重启进程；设置说明这一例外。
- None：不需要 UI 翻译，不因全局切换重启后台任务。

第三方 manifest 的交付是独立扩展阶段，不阻塞当前内置面板双语版本；该阶段完成前不得宣称 V2 插件 i18n 已全部落地。

## 10. 错误与兼容迁移

现有 `Reply::Error { message }` 和 `ServiceError.message` 不能通过匹配中文内容猜测错误码。短期在操作边界提供本地化摘要，例如“无法加载任务”，raw message 仅用于可展开/可复制的诊断详情；已识别的错误使用稳定 code + 参数。

追加结构化错误字段时保留旧 message，使用 serde 默认语义并补兼容测试；不得因本规格全量改写旧线上错误形状。未知错误展示通用本地化摘要，详情原样保留。翻译不能改变错误级别、重试判断或权限判断。

新 GUI 对不支持语言设置请求的旧 daemon 使用本地最后确认快照/系统临时显示，设置展示需升级服务的原因；不静默退回 GUI 自行持久化权威设置。旧 GUI 与新 daemon 的既有业务请求保持可用，未知请求导致的断连由新 GUI 捕获而非崩溃。

## 11. 实施切片与发布门槛

| 阶段 | 交付 | 完成门槛 |
|---|---|---|
| I1 基础契约 | model、纯解析库、英中资源规范、服务、daemon 持久化、设置入口、PanelDescriptor 标题 | 设置到标题的完整纵向链路通过；仅标为部分支持 |
| I2 Host | chrome、菜单、overlay、Toast、错误适配、窗口广播 | Host 自有界面英中覆盖，切换保留工作状态 |
| I3 官方面板 | 按面板迁移 native 与 WebView；优先 Todo、Usage、Code Health，再完成 product 清单全部面板及预览壳层 | 逐个面板具有双语清单及切换证据；硬编码门禁生效 |
| I4 公共应用契约 | bytehost 上游字段/能力、tag、Dozer 适配及一个应用样例 | 安装校验、fallback、Surface 边界与旧应用兼容通过 |

I1–I3 完成才可发布“Dozer 内置界面支持英语/简体中文”。I4 独立验收，不要求迁出所有面板，不新增插件市场。审计清单必须包含所有自有 Web 前端（editor、json-editor、tabular-host、todo-content、usage-content、codehealth-content、group-chat-content、review-trace、plantuml-viewer、flyfish），按自有 chrome 与用户/第三方正文区分范围，不能因未列入试点而漏掉。

## 12. 验收标准

| ID | 场景 | 必须结果 |
|---|---|---|
| L01 | 新装；系统 en-US、zh-Hans-CN、zh-TW、未知及空候选 | 严格按 §4 匹配，未知/无支持候选为 en |
| L02 | 选择 zh-CN 后重启 GUI 和 daemon，再改 OS 语言 | 显式选择保持；System 在下次启动重算 |
| L03 | 配置缺失、损坏、写入失败、服务离线 | 不崩溃；失败不假报保存成功；权威状态不被缓存覆盖 |
| L04 | 两窗口同时提交；两个 GUI 进程；断线重连 | 冲突可见；其他进程正常连接时 2 秒轮询周期内发起同步；重连获取完整快照 |
| L05 | 缺当前 key、缺默认 key、空翻译、重复 key、错参数、未知 owner | 确定性 fallback；资源错误可诊断；无空标题、无 panic |
| L06 | Rust/TS 使用相同测试向量，包括 Unicode、花括号和恶意 HTML 参数 | 查表/插值结果一致；参数不执行脚本 |
| L07 | 有编辑输入、滚动位置、终端会话和运行任务时切换 | 工作状态保留；不重启终端/任务，不修改用户内容 |
| L08 | 所有 overlay 打开；原生/iced 菜单；现存 Toast | 文案更新或菜单安全关闭；动作/快捷键/id/到期不变 |
| L09 | WebView ready 前切换、隐藏后切换、重建、旧包晚到、仅语言改变 | 最终均为新语言；无去重漏推，无旧包回退，无 reload |
| L10 | Code Health/Usage 等计数、日期、金额及服务失败 | 单位/摘要翻译，原始数值/精度/诊断不变 |
| L11 | en/zh-CN + 明暗主题 + 最小窗口 + UI 缩放 | 标签可读，确认按钮/金额不截断，CJK 无方块；溢出有明确策略 |
| L12 | 新旧 GUI/daemon/前端配对，旧第三方应用 | 既有业务可用；不支持语言能力时明确降级 |
| L13 | MCP、日志、存储快照在切换前后对比 | 机器标识、schema、审计原文、项目数据不因 UI 语言改变 |
| L14 | 单面板 + Host 测试组合 | 可独立加载资源与服务，无其他面板依赖 |

自动检查包括：资源双语 key/占位符对齐、Rust/TS 共享向量、locale 服务及 IO 故障测试、WebView 协议测试。新增用户可见硬编码进入 CI 检查，结合明确入口/审计清单及人工 review；不能仅扫描中文，因为英文硬编码同样违规。豁免需标明原因，不能把整个面板排除。

人工验收按 I3 面板清单记录两种语言与关键状态的截图/结果。伪本地化拉长标签可辅助发现布局问题，但不作为第三种发布语言。仅资源 key 数量相等不足以代表界面覆盖完成。
