**byteui 及其在 Dozer、Digger 中的应用：代码分析与优化建议**

分析日期：2026-10-31。依据本地三个仓库源码、Cargo 清单与锁文件、消费方适配层及 CI 配置；为解释间接依赖，补读了本地 bytehost 的相关清单。本文只新增分析文档，不修改业务代码。

**1. 结论与分析边界**

byteui 已经从“共用几个控件”发展成两个产品的视觉与交互基础层。它同时承担设计 token、iced 控件构造、页签与菜单交互、macOS 菜单渲染等职责。当前最值得投入的是完善这些共享契约：缩放合法性、布局与渲染度量一致性、原生窗口生命周期、主题数据源及接口语义。

继续搬运更多产品控件的收益，低于先把这些基础契约收稳的收益。共享层中的一次错误可能同时影响 Dozer、Digger、bytehost-chrome，以及两个产品消费的同源面板。

三个版本事实需要区分：

- byteui 本地 HEAD 为 `1285b9a`，版本 `0.5.0`，与两边 Cargo.lock 中 byteui 的 git 提交一致：`1285b9ac362e6eeaf1d4fa8a9caef7fd3ee62bbc`。
- Dozer 的 `.cargo/config.toml` 将 byteui 覆盖到 sibling 工作树；Digger 的本地配置主要将 Dozer 库覆盖到绝对路径。因此锁文件一致，不能直接证明本地开发构建使用完全相同的来源。首次在 Dozer 目录用 `--manifest-path ../byteui/Cargo.toml --locked` 运行测试，就因继承本地 patch、需要更新锁文件而停止。
- 文档描述的 v0.4.4/85 个测试/没有消费方 CI 已过时。当前有 115 个 byteui 库测试，Dozer 与 Digger 都有 consumer-check workflow 文件。这里只确认配置存在，没有查询远端实际执行记录。

统计口径：只扫描 byteui/src 与两个产品的 crates 下 Rust 文件，含测试、注释；引用次数是字面 `byteui::` 次数，不能当作运行时调用量。byteui/src 有 43 个 Rust 文件；Dozer 456 个 Rust 文件中有 100 个直接提及 byteui，共 1844 次；Digger 115 个文件中有 40 个，共 273 次。Digger 的这组数不包括其 git 依赖中的 Dozer 面板及 bytehost 源码，明显低估实际覆盖面。

**2. 实际架构与复用路径**

| 层次 | byteui 的职责 | 应用中的位置 |
|---|---|---|
| theme | color/font/geometry/icon_size/region，运行时主题与缩放 | 全局视觉、字体、布局、原生子视图定位的基础数据 |
| form/data/layout | 输入、搜索、按钮、选择、卡片、列表、分隔等 | 业务视图组合的基础元素 |
| interaction | 图标按钮、页签内核、窗口化算法、弹层菜单 | 面板 chrome、页签关闭、悬停、溢出导航 |
| feedback | 对话框、提示、toast、曲线加载动画 | 异步反馈与确认操作 |
| native_menu | macOS NSMenu、自定义菜单项、SVG 栅格缓存 | 解决原生 WebView 遮挡 iced 菜单的混合渲染问题 |

依赖关系可以概括为：`Dozer UI / Digger desktop → byteui`；`两产品 → bytehost-chrome/settings-ui → byteui`；`两产品 → dozer-panel-* → byteui`。这些是代码依赖方向，不代表全部面板内容由 iced 绘制；Usage、Todo、Group Chat 等还包含 WebView 页面与独立 CSS。

Dozer 是主要的组件来源与直接消费者：dozer-app 加 10 个面板 crate 共 11 处 Cargo 清单直接声明 byteui。其 theme、chrome 和 host_adapters 将产品区域样式、国际化、悬停动画、窗口系统接入共享组件。

Digger 的直接接入主要集中在 desktop 组合根与 adapters。它既消费同源 Dozer 面板，也通过 PanelDisplay 等接口提供自己的消息路由、区域样式、悬停反馈和宿主能力。例如 `dozer-panel-preview/src/display.rs` 定义展示接缝，两边适配层提供消息包装、页签与手势。

这个分层有实际价值：业务面板不必依赖具体产品的 App；byteui 不必理解 Project、Agent 会话或异步请求代次。Digger 的 `PreviewDisplay::wrap` 携带 project_id/epoch，这是产品生命周期契约，应继续留在适配层。

当前做得好的部分应保留：

- `tab_strip` 的纯逻辑与 iced 视图分开，可独立验证。
- `TabCoreArgs`、`PanelTabArgs` 已用具名参数规避同类型位置参数误传。
- Tooltip 始终保留包装节点，避免控件树切换导致悬停状态丢失。
- `native_menu` 已按图标、颜色、尺寸缓存 NSImage，没有必要再建议“增加图标缓存”。
- `theme::region` 的 resolve 接收显式 token/scale，是逐步减少全局状态依赖的好范例。

**3. 应优先处理的正确性问题**

**P1-A：统一缩放输入校验；已复现。**

位置：`byteui/src/theme/icon_size.rs:109`、`:132`、`:143`、`:174`。

持久化读取会验证有限正数并钳到 0.5～3.0；环境变量只验证有限正数，直接通过 base_scale 返回；set_scale 则直接 clamp。临时副本中的公共 API 探针确认：

- `set_scale(f32::NAN)` 后 `scale().is_nan()` 为真。
- 新进程设置 `BYTEUI_ICON_SCALE=99`，`scale()` 为 99，大于 SCALE_MAX。

NaN 会继续传播到图标与几何尺寸；字号从浮点转整数还可能变为 0。环境变量超大值则可能使界面严重放大。这些是明确的输入契约漏洞，尚未在真实窗口里验证具体视觉故障。

建议引入一个纯函数 `normalize_scale`，明确非有限值、非正值与越界值的策略。环境变量、JSON、token 初始化、set_scale/zoom_by 均经过它；init 时解析一次环境覆盖并写入有效状态，避免每个 accessor 在哨兵状态下重复读取环境。不要把“某些入口钳制、某些入口返回原始值”继续留给调用者理解。

验收：NaN/±∞/0/负数/极小值/极大值、主旧环境变量优先级、重置、持久化及重新初始化。现有 init_scale_with 遇到有效环境值只 return 的设计也应明确：它假设当前运行时尚无 override，重复初始化不会主动清掉旧值。

**P1-B：页签宽度估算与缩放渲染脱节；源码可确认，视觉影响待窗口验证。**

位置：`digger/crates/desktop/src/adapters/tab_chrome.rs:21`；`dozer-app/src/workspace/hook.rs:132`；`dozer-panel-agent/src/tabs.rs:186`；`byteui/src/interaction/panel_tabs.rs`。

两边预览页签都采用 `units * 8.0 + 18.0 + 12.0` 的固定估算；实际标题字号和关闭按钮尺寸读取已缩放 token。scale 改为 2 时，估算值不随之改变。tab_window 虽然算法正确，输入宽度却低估渲染宽度，可能把已被裁掉的页签仍算作可见；自动 reveal 的判断也会受影响。

`字符码点 > 0x2E80 就算全宽` 还是粗略启发式，无法正确覆盖组合字符、emoji 等情况。应先统一 `TabMetrics`，以实际 font size、关闭按钮、prefix/suffix、gap/padding 构造估算；有测量能力时使用与 renderer 相同的 shaping/layout。纯 tab_window 继续接收宽度数组，不应被迫依赖渲染器。测量结果按标题、字体、scale、prefix/suffix 配置缓存。

验收：两种主题与 scale 0.5/1/1.5/2/3、长 ASCII/CJK/emoji 标题、单个超宽 tab、选中隐藏页签及后缀按钮场景。跨缩放验证“选中项在可视区内”，比仅验证公式的固定返回值更有价值。

**P1-C：原生菜单的挂靠窗口缺少生命周期约束；风险，未复现崩溃。**

位置：`byteui/src/native_menu.rs:89`、`:96`、`:109`。CONTENT_VIEW 保存裸 NSView 指针，content_view 把它转换成静态引用。install 的安全函数签名既没有绑定窗口寿命，也没有要求 MainThreadMarker；show 才断言主线程。只有调用约定保证窗口还活着。

Dozer 的 SearchOverlay 等浮窗在 Drop 中手动重新登记主窗口，说明当前正确性依赖产品维护全局单槽。浮窗交叠、销毁顺序变化或未来多窗口接入都容易破坏这一约定。现有 Drop 恢复逻辑应被视为必要的补偿，而非生命周期问题已解决的证明。

优先让 `show_for_window(window, mtm, items, pos)` 在同步菜单期间借用有效窗口，不长期登记裸指针；若必须保留登记机制，则使用有所有权/弱引用约束的上下文，按 WindowId 定位，支持注销，并显式表达主线程要求。仅补一个 clear 函数不足以建立安全契约。

验收：窗口 A→浮窗 B→浮窗 C 的创建关闭顺序、菜单取消、目标销毁后调用、主窗退出。这里不建议用真实悬空指针制造崩溃来验证；先对上下文选择与注销逻辑做纯状态测试，再做窗口集成测试。

**4. 主题系统：先统一数据与更新语义，再考虑性能优化**

**P2-A：分散全局主题导致更新和测试契约复杂。**

颜色、方案名、字体、几何、图标 token 各有全局状态，scale 另用 AtomicU32。`set_scheme` 分两次写颜色与方案名；set_theme 能换颜色但不更新方案名。这是当前 API 明确允许的状态，而宿主/网页/终端又会读取 scheme，需定义自定义主题与色彩模式之间的关系。

Dozer 中 `theme::color`、`theme::font` 等直接引用很多；每次 current 取得锁并复制 token。这里可以提出优化方向，不能在没有 profile 的情况下断言锁已是性能瓶颈。

建议提供包含 scheme、colors、font、geometry、icons、scale、revision 的 UiTheme 快照。先增加 snapshot 与纯 resolve 接口，在单个视图构建时获取一次；现有全局 accessor 保留为兼容层。随后统一替换主题的入口，避免逐模块发布中间态。快照中先保留原始 token，由明确的度量函数缩放，避免混淆基准与最终值。

如未来确实需要每个窗口独立主题，再让上下文进入窗口/视图树；当前两个独立进程共享“库”，并不共享同一份运行时 static，不必为了跨进程复用提前做复杂状态服务。

**P2-B：外壳 region 仍有两个来源，但已有 CI 守卫。**

Dozer `theme/region.rs` 仍解析 workspace.json 中的外壳和产品区域；byteui 的 region 用 Rust 常量保留共享外壳。context_menu 已委托到 byteui，不能再作为“仍完全重复”的例子。当前脚本比对的 8 个区域及窗口背景全部一致。

Dozer consumer-check 已运行 check-shell-regions，旧 unfinished 文档中的“没有 CI”需要更新。但 workflow checkout byteui 未指定 ref，对比对象是其默认分支，而非 Dozer 锁定的提交；byteui 自身 CI 也没有反向编译两个消费方。脚本在找不到 sibling 时还会成功跳过。

建议让 Dozer 的共用外壳 accessor 委托给 byteui，Dozer 独有面板区域继续留在产品。迁移时同步原生子视图几何，保留 JSON 自定义/覆盖能力的明确选择，不能只改视觉 getter。过渡期 CI 对比锁文件对应的 byteui 提交，并提供必需模式，缺依赖时失败。

**P2-C：Rust token 与网页 CSS 继续分叉。**

Usage、Todo、Group Chat 等网页各自维护深浅色 CSS。Todo 中 dark `--body=#c9d4dc`、`--red=#FF5C5C`，byteui 同名 token 为 `#9AB4C4`、`#FF6E6E`。差异可能有可读性设计意图，本文不把它们全部判成错误；但同名不同值会隐藏覆盖来源。

Digger settings/theme.rs 已手工从 current tokens 构造第一方审阅页 CSS，说明桥接能力有实际需求。建议从同一份主题描述导出基础 CSS variables，产品覆盖使用明确的语义名字或单独 override 文件。网页组件样式留在各面板，基础色板无需重复手抄。只对产品控制的第一方页面应用主题。

**P2-D：持久化错误处理已经出现第三份实现。**

byteui 的 persist_scheme/persist_scale 静默忽略 IO 错误；Dozer extensions/settings.rs 与 Digger settings/theme.rs 为让用户看见保存失败，分别实现 scheme JSON 和目录/文件写入逻辑。

保留产品对路径与提示的所有权，但在共享层提供显式路径、返回 Result 的读写 API；兼容的静默函数可以包装它。使用临时文件加原子替换，防止半写文件。这样共享的是文件格式与可靠写入，具体 UI 文案仍由产品决定。

**5. 组件 API 与交互契约**

**P2-E：清理 tooltip 已失效的控制接口。**

`feedback/tooltip.rs:24` 的 controlled_tooltip 接收 `_show` 却完全忽略。稳定树结构与 iced 自带两秒 delay 是合理修复；问题是 PanelTabArgs、PanelDisplay 和两产品仍携带 show_tooltip/hover_tooltip_ready，以及相应时间记录和测试。

应将接口命名为 delayed_tooltip，并以 TooltipOptions 表达 delay/position。先弃用旧接口，再删掉只服务该开关的计时和消息；逐个检查用途，悬停高亮仍需要保留。需要真正禁用时，用稳定的包装结构表达，避免重新引入条件包装导致的 diff 问题。

当前 Digger 的 PreviewDisplay 已返回阶跃 hover 0/1、产生 PreviewHover 消息，但模块开头仍写恒 0/Noop，tab_chrome 注释也有同类陈旧描述。应先按实现更新文档，避免下次重构误删有效交互。

**P2-F：把布尔位置参数升级为具名配置。**

input_text 有多组 size/bg/suffix/framed/bare 包装；icon_button_entry 有 active/dim/card/interactive 等连续参数。虽然编译器能确认类型，无法防止布尔含义与顺序写错。TabCoreArgs 已展示了更好的做法。

建议增加 InputOptions、IconButtonArgs；用 InputAppearance 等枚举取代可形成模糊组合的布尔参数。prefix/suffix 统一为 slot；事件闭包保留适合的泛型，不为整齐而一律 Box。旧函数委托新实现，迁移两产品后再在后续破坏性版本删除。共享面板已有不少 boxed 展示接缝，其成本应先测量，不需要与本次 API 清理同时全面改写。

search_box 还写死了“搜索”tooltip，应增加可传文案；普通输入与 text_area 对 bare 的 padding 语义也不同，应明确 bare 是仅去边框还是连内距一起交给容器。

**P2-G：补齐控件缩放与样式覆盖，而非一次性 token 化全部常量。**

tooltip 用固定 size(12)，select 没有像 input_text 一样显式设置字号，也没有统一下拉菜单样式；多个控件的 padding/radius 是常量。字体按全局 scale 变化时，这些度量需要有明确策略。

先提取两个产品确有差异或频繁变化的组件级指标，如 FieldMetrics、TabMetrics、MenuMetrics。描边是否缩放、命中框是否有最小尺寸应逐项定义，不宜把所有常量机械乘 scale。region 的普通边框固定、maximize overlay 边框缩放，目前有测试明确锁定这种差异，不能当作无意漏乘直接修改。

**6. 构建、依赖与测试优化**

**P2-H：先切断构建脚本对 UI 面板的依赖。**

Digger desktop/build.rs 只为取得资源路径，却通过 build-dependencies 引入 dozer-panel-usage/todo；这些面板又依赖 byteui/iced。Digger 根清单已经为 release host 构建单元设置 debug-assertions，以绕过 iced_renderer 没有后端时的检查。这比“组件库文件有点大”更明确地暴露了构建边界问题。

建议将资源路径/清单拆成不依赖 UI 的资源小库，或由构建资源工具产出明确清单，build.rs 仅依赖它。验收 host/target 分离的 release 构建，然后撤销该 workaround。byteui 的 canvas/native-menu 可在确有轻量消费者后再变 optional feature；若每个产品本来就用这些功能，拆 feature 不会自动让最终 App 大幅变小。

Dozer 的 11 处 byteui 声明应集中到 workspace.dependencies，消费 crate 用 workspace=true，与 bytehost 已采用的做法一致。共享依赖守卫保留；新增消费矩阵验证 byteui 候选提交与 bytehost/Dozer/Digger 的兼容性。锁定依赖构建与覆盖候选源码构建应作为两个独立检查，后者使用临时锁文件，避免污染正式锁文件。

**P2-I：测试重点从“能构造”推进到行为契约。**

现有测试不是空白：有颜色、原生菜单栅格、溢出算法、tooltip 树结构、按钮状态等测试；但 form/data 的部分测试只验证构造不 panic。byteui CI 是 macOS fmt/clippy/test，尚无渲染快照矩阵。

font 和 icon_size 的测试会修改全局 token/scale，缺少跨模块统一隔离；color/geometry 各自的锁也不能保护其他模块的读取。连续 100 次、16 线程执行现有 115 个测试均通过，因此这里记录的是源码可见的交错风险，没有宣称已复现 flaky。优先将验证转为显式 token 参数的纯函数；保留少量全局兼容接口测试并统一隔离，或放入独立测试进程。

建议新增的高价值验证是：非法缩放输入、页签实际布局与 reveal、菜单边缘可达性、深浅色与多缩放的关键控件截图、多窗口菜单归属。菜单 rows_size 使用文字高度估算，而 shell 用实际布局；长菜单需要检查底部可达性，并在确实超高时约束高度和滚动，不只钳制左上角。

**P3：性能项以测量触发。**

- 先测文件树/多页签 view 构造、布局、渲染耗时，再决定快照能否改善重复 current 读取。
- math_curve AnimatedLoader 在每次 redraw 后请求下一帧，应测加载页的空闲 CPU/GPU；再决定限帧、不可见时暂停或 reduced motion。源码证明会持续请求重绘，不等于已测得高功耗。
- SVG 已有 renderer 路径和原生菜单缓存，不预设“每次 view 都解析 SVG”是瓶颈。
- 当前 data::list 只是行组件。需要长列表优化时应先定位实际列表调用链，再选择虚拟化；不能把增加一个复杂 Table 当作当前首要任务。

**7. 推荐的落地顺序**

| 批次 | 改动 | 验收标准 |
|---|---|---|
| A，局部正确性修复 | normalize_scale；页签度量统一并随缩放变化；修正文档 | 非法输入测试通过；多缩放 reveal/布局验证通过 |
| B，原生菜单生命周期 | 显式窗口上下文、主线程约束、销毁处理 | 浮窗交叠与关闭顺序正确，无全局裸指针寿命假设 |
| C，兼容式接口收敛 | snapshot/纯 resolve；TooltipOptions；InputOptions/IconButtonArgs | 两产品保持交互一致；旧 API 可平滑迁移 |
| D，数据和构建边界 | 共享 region 委托、CSS 导出、Result 持久化、资源构建依赖拆分 | 单一来源可追溯；保存错误可见；release 无后端检查 workaround |
| E，长期守卫 | 候选提交消费矩阵、关键视觉基线、按测量优化热路径 | 发布前验证覆盖两产品与 bytehost，性能收益有数据 |

不建议在本轮引入 amis 式 JSON UI 引擎、通用完整 Table、跨框架组件运行时，或把 Project/Agent 产品状态搬进 byteui。等另一个 UI 框架确有消费需求，再考虑独立的框架无关 tokens crate；目前可以先统一主题导出，不必为此一次性拆分整库。

**8. 本次执行的验证**

- 在临时目录复制 byteui，绕开 Dozer 当前本地 patch，执行 `cargo test --locked --offline --lib`：115 passed。构建产物放在 `/tmp/byteui-review-target`。
- 对编译出的测试二进制执行 100 次、每次 16 线程：全部通过。此操作用于核对已发现的全局状态交错风险。
- 仅在临时副本新增公共 API 缩放探针，分别启动进程验证 NaN 与环境变量 99：均确认上述当前行为。
- 使用临时 byteui 副本运行 Dozer `scripts/check-shell-regions.sh`：8 个区域与背景比对通过。
- 未运行 Dozer/Digger 全量测试、真实窗口交互、GPU 基准或远端 CI；页签视觉故障、菜单生命周期风险和性能推断均据此标明限制。
- 原有 Cargo.lock、dozer-app/Cargo.toml、Info.plist 工作区修改保留；分析未修改这些文件，也未修改 sibling 仓库源码。
