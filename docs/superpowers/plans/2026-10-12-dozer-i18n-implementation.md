# Dozer 多语言支持实施计划

> 日期：2026-10-12。状态：Task 01 已完成（清单与生产入口基线），其余任务待实施，未完成的复选框仍未勾选。
> 规格：[多语言支持 spec](../specs/2026-10-12-dozer-i18n-design.md)。
> 补充要求：必须将当前代码中属于界面文案的中文迁入中文语言包，并替换原调用点；不能只为新增功能添加翻译接口。
> 本次交付为计划，不执行功能实现、依赖发布或打包。以下新增文件、命令入口均为待实现项。

## 0. 执行约定与最新代码校准

- 实施前记录 HEAD、工作区改动和工具链版本；保留其他任务 WIP，不重置或覆盖。每项任务记录文件、验证命令、结果和残留问题，未验证不打完成勾。
- 依赖方向遵循 spec：DTO 在 model，纯解析在 i18n crate，服务在 services，传输在 core/client，持久化在 daemon，展示在调用方。面板不通过 `App` 或直接依赖 client 获取语言。
- 当前工作区正在迁移 Usage：已出现 `crates/dozer-panel-usage/`，前端与 assets 正从 `dozer-app` 移入该 crate。Task 01 必须重新确认唯一生产入口，不能恢复已删除目录或维护两份 Usage 译文。
- 因上述变化，spec §5 的“资源位于 dozer-app/locales/panels”仅适用于尚在 app 内的面板。独立面板资源跟随所有者，Usage 使用 `crates/dozer-panel-usage/locales/{en,zh-CN}.json`，不得反向依赖 app 的目录。实施 Task 01 同步修订 spec 路径说明；本计划不改变 fallback 或资源共享语义。
- 复用现有 SurfaceExecutor。Todo 等自有 envelope 与 Usage/CodeHealth 的共享 Surface 接入分别核对，不假定所有面板共用同一种推送协议。
- 每个任务形成可审查的修改组；独立面板与上游公共库分清归属。任务依赖不要求多 agent 执行。

## 1. 存量中文迁移的硬性规则

### 1.1 完成定义

每条存量用户可见中文都要得到可追踪结论：迁移至 `zh-CN.json` 的某个 key，或有明确非 UI 理由的保留项。迁移完成意味着资源存在、代码调用已替换、中文显示经过核对、英文翻译存在，四者同时成立。

中文包以当前实际中文文案为初始值，保留语义、标点、换行及参数位置。纠正明显文案问题须在清单中记录旧值/新值/原因，不能借迁移批量改产品措辞。英语包使用同一组 key。源代码不保留 `t(key, "中文")`、中英文分支或中文 fallback 作为第二份中文词表；官方 fallback 为英文安全文案，正常中文只从中文包解析。

动态句子迁移整句模板，例如现有 `format!("已删除 {} 个文件", count)`：

```json
// zh-CN.json 示例（实际 JSON 文件不含注释）
{"files.delete.completed": "已删除 {count} 个文件"}
```

对应英文按数量选择 spec 规定的变体；若使用 `.one/.other`，中文资源也具备相同 key 集。不要拆成“已删除” + 数字 + “个文件”，不要通过中文原文反查 key。

同一文字在不同语义位置可以有不同 key（例如删除文件和删除会话），不要仅凭文本相同全局合并。同一面板在 Rust 与 TS 中的同一语义只维护一份资源。

### 1.2 盘点边界

覆盖 Rust 字符串及 raw string、`format!`、状态 `label()`/`Display`、HTML 内嵌文案、TS/TSX/JS 文本、placeholder/title/aria-label、CSS `content` 中的可见文字、Rust 拼接的 WebView payload、错误及 Toast 产生点、原生菜单和 OS 对话框的应用传入文案。

中文注释、开发期 assert/expect 诊断、机器日志、协议稳定值、Agent prompt、用户/测试示例内容单独分类，不自动翻译。测试 fixture 中模拟 UI 的文案需随协议迁移，模拟用户内容的中文保留。一个字符串同时用于日志和 UI 时必须拆开用途，不因“也写日志”而豁免界面。

生成的 `assets/*` 通过构建源文件更新，不手改 bundle；`node_modules`、第三方 stdlib/upstream 正文不机械搬入资源。若其中确有自有界面在使用的可见控件文案，记录具体组件及语言 API/适配方案，不能整目录豁免后忽略其显示结果。

### 1.3 迁移记录与门禁

拟新增 `docs/dozer-v2/i18n/`：

- `baseline.md`：代码/构建入口、版本、已有失败、资源所有者。
- `inventory.json`：每条候选的稳定记录 ID、源路径/符号/文字、用途分类、owner/key、任务、状态及验证证据；行号只作定位辅助。
- `glossary.md`：中英术语、品牌保留项、措辞修订。
- `ledger.md`、`acceptance.md`：任务与 spec L01–L14 验收证据。

总账统计必须满足：候选总数 = 已迁移 + 有理由排除 + 未解决。最终范围内未解决和残留硬编码均为 0；禁止删除基线记录让数字归零。已迁移 key 必须有资源且仍有使用点，扫描规则不宣称能自动证明全部 UI 覆盖。

## 2. 阶段与依赖

| 阶段 | 任务 | 出口 |
|---|---|---|
| I1 契约与首条链路 | 01–05 | 存量清单、双语解析、daemon 设置、Settings→Rail 热切换 |
| I2 Host 全量迁移 | 06–07 | chrome/overlay/Toast/错误展示迁移 |
| I3 内置面板与预览 | 08–13 | 所有原生及 Web 界面迁移、打包、全量验收 |
| I4 第三方契约 | 14 | bytehost 能力及消费验证，独立于内置版本发布 |

依赖：01 → 02 → 03 → 04 → 05；06/07 依赖 05；08 依赖 02/05；09 依赖 07/08；10/11 依赖 06/07/08；12 依赖 09/10/11；13 依赖 06/07/12；14 依赖 02/05/08，且需先核对上游当前契约。每个迁移任务同时交付中文包、英文包、调用替换及证据，不把中文抽取拖到最后。

## 3. 实施任务

### Task 01：建立逐条文案清单与生产入口基线

**读取：** `product.rs`、`panel_registry.rs`、全部 `src/{app,workspace,chrome,platform,extensions,preview}`、两个 HTML host、所有 Web 包、`assets.rs`、打包脚本与独立面板目录。
**新增：** §1.3 的记录文件；`scripts/i18n/scan_sources.py`（候选扫描工具）。

- [x] 记录实际工作区 revision/WIP、现有测试结果及 package scripts；定位 Usage 迁移后的真实构建与资源引用。
- [x] 扫描全部自有生产源文件中的中文候选，并检查用户可见英文；对 multiline/raw string、模板字符串、JSX、HTML 属性分别处理，避免把注释行数当文案数。
- [x] 沿 view/presenter、payload、错误和 Toast 调用链人工分类，建立 owner/key 映射；列出所有无静态字面量但可显示文本的入口。
- [x] 为每个面板、overlay、HTML host、自有 Web 包分配 Task；动态新文件须重新进入清单。
- [x] 确定 Rust/TS 资源单一来源、共享 TS 解析模块的打包边界；独立面板不得依赖 app。若 Usage 库需要共享 TS 包，将通用解析器移至中立目录并同步 spec，不复制实现。
- [x] 对迁移中的路径与规格差异写回 spec；记录当前版本 schema 兼容测试夹具，不把未提交代码当稳定发布版本。

**验收：**无未分配的 UI 区域；候选有分类/负责人任务；未迁移清单可重跑。尚未审计项保持 unresolved。

### Task 02：双语资源、解析器和资源校验

**新增：** `crates/dozer-model/src/i18n.rs`、`crates/dozer-i18n/{Cargo.toml,src/lib.rs,tests/}`、Host/面板 locale 目录、共享 TS 解析模块、`scripts/i18n/check_resources.py`。
**修改：** model 导出及相关 Cargo 依赖；workspace 现有 `crates/*` 自动纳入新 crate，无需重复枚举。

- [x] 定义 Locale、LocalePreference、LocaleSnapshot、LocalizedText、具名参数；用具名请求结构代替易混淆的同类型位置参数。
- [x] 先写 fallback、空值、模板错误、转义、缺参数、未知 owner、恶意 HTML、Unicode 的共享 JSON 测试向量，再实现 Rust/TS 同规则解析。
- [x] 建立最小 Host 词表：语言设置、面板标题、通用保存失败。现有中文迁入 zh-CN，同 key 补英文；使用 Typed key/常量或受检查的字面量，避免拼接动态 key 无法审计。
- [x] 校验重复 key、空字符串、非字符串值、占位符/key 集差异；duplicate key 必须在 JSON 反序列化覆盖前发现。
- [x] 提供目录/owner 注册方式，使独立面板可随自身资源使用解析器；编译期嵌入 Rust 资源，前端静态 import，运行离线。
- [x] 明确格式化、复数变体及术语规则；不把 UI locale 接入 PTY 环境或 MCP。

**验收：**Rust/TS 共享向量同结果；注入缺 key/参数不一致会检查失败；中文展示确实来自 zh-CN 资源。对应 L05/L06。

### Task 03：语言持久化、服务与兼容传输

**修改：** `dozer-services/src/lib.rs`、`dozer-client/src/lib.rs`、`dozer-core/src/{protocol,paths}.rs`、`dozerd/src/{lib,main,server}.rs`。
**新增：** `dozerd/src/locale_settings.rs` 及服务/协议测试。

- [x] LocaleService 接入 Services 及全部真实/假服务构造点；面板不新增 `dozer_client` 引用。
- [x] 追加 get/set 请求与带 snapshot 的冲突结果，保留旧 Request/Reply 形状；旧服务不支持新请求时客户端明确分类。
- [x] 实现 daemon 串行 CAS、原子持久化、配置缺失/损坏恢复；注入文件系统失败测试，不依赖 chmod 在不同环境中的偶然效果。
- [x] 无变化的偏好及解析结果不推进 revision；revision 冲突不覆盖。处理计数溢出为明确失败，不回绕。
- [x] 启动 System 刷新采用读取后带 expected_revision 的提交；显式偏好不被陈旧探测回包覆盖；同一配置恢复流程只由 daemon 写入。
- [x] 测试旧业务请求仍可用、旧 daemon 拒绝/断连新请求、新客户端降级，不自动进入无界重试。

**验收：**离线/IO 失败/并发写/重启恢复通过；用户项目数据与终端配置不变。对应 L02/L03/L04/L12。

### Task 04：App 语言快照、系统探测和窗口同步

**新增：** `dozer-app/src/i18n.rs`、`platform/locale.rs`。
**修改：** `app/{app,state,message,update}.rs`、`platform/{mod,window_events}.rs`、服务适配与启动入口。

- [x] 在平台隔离层读取有序系统语言偏好，核对实际 OS API；按 spec 处理 script/region 优先级并测试 `zh-Hans-TW`、`zh-Hant-CN` 等冲突输入。
- [x] 建立 App 快照、连接 epoch、请求序号和本地 display generation。后台迟到回包必须在应用时检查 epoch/序号，不能覆盖刚确认的新语言。
- [x] 缓存仅写最后确认快照；断线使用缓存/临时系统值，重连以 daemon 为准。连接换代重新建立 revision 基线。
- [x] 每 2 秒周期发起一次跨进程读取（有请求在途则合并），获焦立即刷新；窗口内部通过单次 App 状态更新广播，不各自轮询。
- [x] 只有 active locale 改变才推进 display generation，避免无变化轮询导致重绘/推送风暴。
- [x] 验证文案上下文可注入独立面板与单窗口测试宿主，不引用全局可变 App。

**验收：**正常网络/服务延迟下按约定周期发起同步，断线与乱序测试通过，两个窗口一致。对应 L01/L04/L14。

### Task 05：设置入口与面板标题纵向交付

**修改：** `extensions/settings.rs`、`platform/settings_overlay.rs`、`product.rs`、`panel_registry.rs`、`chrome/rail.rs` 及标题消费点。

- [x] 将现有面板标题中文逐项原样搬入语言包，加入设置语言页的全部英中文案。
- [x] PanelDescriptor 保存 key/fallback，不在 OnceLock 中冻结译文；替换要求静态字符串的消费签名，不使用 Box::leak。
- [x] 增加 System/English/简体中文、保存中/失败/冲突/服务不支持状态；保存期间防重复提交，确认成功才更新选择。
- [x] 设置页自身和已打开面板标题同步变换，保存失败沿用旧语言；保留当前设置 tab、输入及窗口状态。
- [x] 更新 inventory，记录标题/settings 已清零，不将其他尚未迁移区域标为支持双语。
**验收：**首次选择→daemon→多窗口→重启恢复真实链路通过；I1 仅宣布部分支持。

> Task 05 收尾说明：设置页渲染文案（tab/标题/主题行/Git 连接流程/高级区块/语言页）已全部经 `host_text` 走语言包；`panel_registry.rs` 的 4 条构造器不变量错误（仅 `.expect` panic，不面向用户）在 `build_inventory.py` 的 `NON_UI_STRINGS` 里显式排除并注明理由；`settings.rs` 剩余 3 条（断开账号两处 Toast 文案 + 保存本地记录失败文案）属 Toast/错误呈现迁移，归 Task 07。

### Task 06：Host chrome、菜单和全部原生 overlay

**修改：** `chrome/`、`workspace/` 与 `app/` 中展示入口、`menu_spec.rs`、`platform/*_overlay.rs`、`extensions/footbar.rs` 等。

- [ ] 按 inventory 迁移 Rail/Tab/顶部底部状态、tooltip、空态、搜索、项目创建/删除、文件移动、SSH/数据库配置、确认框、历史及待办详情 overlay。
- [ ] 连同 placeholder、窗口标题、无障碍标签、文件选择器传入文案一起迁移；稳定 action/快捷键/ID 不变。
- [ ] 原生/iced 菜单共用本地化 MenuSpec，处理已打开原生菜单安全关闭；可翻译标题不能成为查找 action 的键。
- [ ] 清理缓存译文：存 key/参数或缓存键带 display generation；切换后所有独立窗口请求重绘。
- [ ] 用长英文和中文检查布局及 CJK shaping，不改用户项目名、路径和动态 Tab 名称。

**验收：**Host 范围所有清单项有资源和调用点证据；L08/L11 覆盖，两种菜单后端均有适当测试。

### Task 07：Toast 与错误呈现迁移

**修改：** `extensions/toast.rs`、`platform/toast_overlay.rs`、App 的 push_toast 入口、各面板错误调用点及必要的 service DTO。

- [ ] 将 UI Toast 文案从裸 String 迁至 LocalizedText（过渡期原文变体只用于有理由保留的外部内容），raw diagnostic 单独保存。
- [ ] 迁移所有中文一次性提示与持续错误摘要，建立操作语义 key，禁止按服务 message 的中文文本匹配错误类型。
- [ ] 日志记录稳定事件与诊断，渲染使用当前快照；保留旧日志原文。
- [ ] 验证切换只改变显示，不新增通知，不更新 id/去重键/expire，不额外写日志；含敏感参数的缺译日志不得打印参数。
- [ ] 需要新增结构化错误字段时先补兼容测试，不顺手改写所有旧 Reply；未知错误有可复制原始详情。

**验收：**L03/L08/L10/L13；中文 UI 错误没有因属于服务层而漏迁。

### Task 08：WebView locale 传递与资源打包基础

**修改：** `preview/webview_protocol.rs`、各 `extensions/*/protocol.rs`、`surface.rs`、前端协议/入口与实际 Surface 适配点。

- [ ] 按当前代码确认每种 envelope 与 PushState；为业务 payload 加语言上下文包装，使去重比较包含 locale/generation。
- [ ] 自有 envelope 追加兼容字段并递增自身 revision；公共 executor 保持框架中立，若仅包装产品 payload 就能解决，不修改公共 API。
- [ ] ready/隐藏恢复/销毁重建发送最新完整快照；旧 Surface generation、旧包、旧异步请求不能回退 locale。
- [ ] 前端以响应式 locale 信号重新渲染，不只在模块初始化时调用一次 t；设置 html lang，不 reload。
- [ ] 翻译资源并入现有 build.mjs，资源缺失令构建失败；从任意构建 cwd 可定位资源，不依赖兄弟工作区运行路径。
- [ ] 给没有正式 envelope 的自有 HTML/预览 chrome 接入最小语言消息，不能把文案注入用户页面；迁移下阶段完成。

**验收：**L06/L09/L12；业务 payload 不变时的语言推送测试必须通过。

### Task 09：Todo、Usage、Code Health 三个首批面板

**文件：** Todo/CodeHealth 原生目录与各自 web 包；Usage 以 Task 01 确认的 `dozer-panel-usage/src/`、`web/usage-content/`、`locales/` 为准。

- [ ] 每个面板单独完成“盘点→中文抽取→英文补齐→调用替换→双语测试→清单关闭”，不要一次巨型替换后才测试。
- [ ] Todo：状态/筛选/分类操作/详情/拖拽提示/确认、WebView 空态及菜单；保留任务文本与用户分类名。
- [ ] Usage：原生加载/错误状态、统计单位、图例、tooltip、空态及 Web 图表；数字/金额/聚合与数据排序不变。
- [ ] Code Health：原生壳、分类/严重度/扫描状态、`view_model.rs`/`protocol.rs` 已拼中文的计数及详情；追加语义字段并由渲染端翻译，保留旧字段兼容。
- [ ] 独立 Usage 库自行携带资源，locale 由 Host 注入；单面板测试宿主不需要依赖 dozer-app/daemon 才能渲染。
- [ ] 更新 UI fixture/渲染 smoke 测试，中文用户内容 fixture 保持不变；验证切换后的状态保留。

**验收：**三个面板每项源中文均对应 zh-CN key；原生和 Web 同步切换，L07/L09/L10/L14 通过。

### Task 10：其余官方面板与辅助界面

**修改：** `extensions/{files,project,database,ssh}/`、`ssh.rs`、`git_log.rs`、`browser.rs`、`conversations.rs`、`group_chat/`、`agent_context.rs`、历史/搜索/应用日志等相关模块及 Agent 在 app/workspace 中的视图。

- [ ] Files/Project/Git：工具栏、空态、上下文菜单、提交/变更/历史说明、创建及删除流程，包含跨 overlay 的同一操作。
- [ ] Agent/Conversations/GroupChat：会话状态、发送控制、错误、上下文操作、群聊 Web 控件；不翻译消息正文、摘要和 prompt。
- [ ] Database/SSH/Browser：连接/驱动/SFTP/书签/导航状态与失败说明；查询结果、远端输出、网页内容不变。
- [ ] Settings Apps/App Host/日志查看等补充区域：自有壳和状态提示迁移；第三方 manifest 显示内容依现有 fallback，不假称已支持 I4。
- [ ] 按 product 清单和额外工具界面对照 inventory，防止只覆盖默认 Rail 面板。

**验收：**每组独立核销，既有业务动作回归通过；未完成任何一组不得宣布 I3 完成。

### Task 11：编辑/预览 Web 与 HTML 模板迁移

**修改：** `web/{editor,json-editor,tabular-host,group-chat-content,review-trace,plantuml-viewer,flyfish}`，`src/{html_host,image_annotate_host}.html`，`preview/` 原生 chrome 与对应资源构建脚本。

- [ ] 迁移源文件中自有按钮、状态、搜索、复制、图片标注、媒体查看、表格页数、错误提示、title/aria-label 与 HTML 模板固定文案。
- [ ] 预览工具文案使用独立 owner（例如 dozer.preview）由 Host/预览层共同加载，用户内容不进入词表。
- [ ] 中文渲染文档、代码、SQL、PlantUML 图内文字保持原文；不翻译编辑器语法 language token，不篡改外部站点。
- [ ] 对第三方控件自带 UI 核对语言配置 API；如需公共组件支持，在上游提出具体改动与 tag，不复制 vendor 实现。
- [ ] json-editor、tabular-host 当前无 test script：为语言消息/状态保留添加最小有价值测试入口，不能执行不存在的 `npm test` 后忽略失败。
- [ ] 确认 flyfish 的 build:check 以及 PlantUML 离线扫描继续通过，不破坏 CSP/离线资产。

**验收：**预览内容前后相同、选择/滚动保留，自有 chrome 双语完成，无残留 HTML 中文硬编码。

### Task 12：硬编码门禁与资源构建收口

**新增：** `scripts/i18n/check_migration.py`、`scripts/check-i18n.sh`、检查器测试与精确豁免表。
**修改：** 实际仓库构建/验证入口、`scripts/build-macos-app.sh`、需要重建的 assets。

- [ ] 检查 inventory 中已迁移原文不再作为 UI 字面量/fallback 存在，key 有资源与引用；新候选必须分类，未分类阻断。
- [ ] 对字符串/JSX/属性建立扫描回归样例，包含注释与 raw string、多行模板，避免用简单中文 grep 宣称无遗漏；英文 UI 字面量通过入口规则及人工清单共同检查。
- [ ] 豁免限定具体文件/符号/文本/理由，禁止整面板或整个错误模块豁免；注入一条中文 UI 硬编码、缺译 key、参数不一致，门禁都应失败。
- [ ] 所有自有 Web 包重建 assets，确认打包消费新 bundle；禁止运行时读取仓库 locales 目录。
- [ ] 当前未见根 `.github/`，先接入实际验证/打包脚本，不虚构现成 CI；若需要新增 CI，复用同一 check-i18n 入口。
- [ ] 将机械源迁移残留清零；保留用户内容/日志等理由明确的中文及历史记录，不改中文注释以改善统计。

**验收：**资源检查、迁移检查及反向注入测试通过；干净构建可复现，离线打包运行能切双语。

### Task 13：内置双语完整验收与交付

**修改：** `docs/dozer-v2/i18n/{ledger,acceptance}.md`；必要时 spec 的实现偏差说明。

- [ ] 按 L01–L14 逐条记录命令/截图/平台/结果，不能以自动检查替代 GUI 热切换验收。
- [ ] 逐面板验证 en/zh-CN、明暗主题、最小窗口与缩放；主窗口/overlay/菜单/WebView/Toast 均覆盖。
- [ ] 验证磁盘写失败、daemon 离线与重启、两个 GUI、旧版本组合、WebView ready 与迟到包；所有 UI 状态保留。
- [ ] 对比切换前后的用户数据/协议身份/终端环境，确认语言改变只影响展示。
- [ ] 运行 §4 的受影响测试和最终工作区检查，区分原有失败/新失败；修复回归，不以提高 baseline 隐藏。
- [ ] 核销 inventory：UI 未解决=0、残留硬编码=0、官方两包 key/占位符一致；证据全部齐全后才标 I1–I3 完成。

**验收：**交付清楚说明内置界面覆盖与 OS/外部应用语言例外，I4 未完成则保持独立待办。

### Task 14：bytehost 应用 i18n 契约及 Dozer 消费（独立阶段）

**归属：** manifest/资源校验/公共语言能力在 bytehost 仓库；Dozer 只改适配、依赖 tag 及样例验收记录。

- [ ] 先核对上游 schema/协商/标题形状，追加可选 i18n/title_key 字段及 fallback，测试旧应用可安装/启动。
- [ ] 加资源路径越界、符号链接、大小、格式和 owner 隔离校验；不允许应用覆盖可信 Host 文案。
- [ ] 按 Surface 支持初始 locale 和协商后的变化通知；External 仅传 DOZER_UI_LOCALE，未声明热更新者不强制重启。
- [ ] 用一个英中应用、一个缺翻译应用和一个旧应用验证；Dozer 安装管理界面与应用内部界面分开验收。
- [ ] 发布动作按实施时授权进行；联调 patch 不提交，正式引用实际发布 tag。上游未就绪不得在 Dozer 内另造 manifest 运行时。

**验收：**I4 单独报告完整性及兼容性，不阻塞已完成的内置双语版本。

## 4. 验证命令与执行频率

以下命令供实施时使用，本次写计划未执行功能测试。新增脚本/新 crate 先完成对应任务再运行。

| 时机 | 命令/工作目录 | 用途 |
|---|---|---|
| Task 02 | `cargo test -p dozer-i18n -p dozer-model` | 解析/DTO；另执行共享 TS 向量测试 |
| Task 03 | `cargo test -p dozer-services -p dozer-client -p dozer-core -p dozerd` | 服务、持久化与协议兼容 |
| 原生状态改动 | `cargo test -p dozer-app` | 实际状态/推送/Toast 行为 |
| Usage 迁移 | `cargo test -p dozer-panel-usage`（以 Task 01 确认名称为准） | 独立面板及依赖边界 |
| 每个受影响 Web 包 | 在其 package.json 所在目录运行 `npm run typecheck`、已存在或新增的 `npm test`、`npm run build` | 类型、行为和资产；不假设有顶层 npm workspace |
| flyfish | 在其目录额外运行 `npm run build:check` | 产物一致性 |
| PlantUML | 其 build 已包含 scan-offline，必要时 `npm run scan` | 离线资源完整性 |
| 每个迁移任务 | `python3 scripts/audit/check_panel_boundary.py`、`bash scripts/check-i18n.sh` | 不新增面板依赖违规，资源与存量迁移检查 |
| 最终收口 | `cargo fmt --all -- --check`、`cargo test --workspace`、`cargo clippy --workspace --all-targets`、`git diff --check` | 整体回归及格式；记录既有失败 |

不为每条静态文案单独写镜像测试；用资源完整性检查、代表性渲染测试和逐区域人工验收保证迁移。持久化、乱序、fallback、Surface 生命周期等行为必须测试。任务内受影响验证通过后继续推进，完整 workspace 验证留在收口或发生跨 crate 风险时执行。

## 5. 完成条件

计划执行完成的证据包括：可追溯中文迁移清单、按 owner 交付的 zh-CN/en 资源、已替换的全部自有 UI 调用点、双语热切换与恢复测试、打包资产验证、L01–L14 验收记录。单有 locale 设置、翻译函数或两份 JSON 文件，均不算完成多语言迁移。
