# ByteUI 跨产品优化实施计划

日期：2026-10-31。状态：P0/P1/P4/P5/P6/P7 的主要代码与自动化测试已实施；
P2 生产调用已迁移到显式窗口 API（人工多窗口验证待做）；P3 依赖图重复已解除，最终 locked 全量构建待 ByteUI 新 tag。详见
`docs/analysis/2026-10-31-byteui-optimization-validation.md`。

Spec：[ByteUI 跨产品优化设计](../specs/2026-10-31-byteui-optimization-design.md)。验收编号以 spec 为准。

## 执行约束与顺序

- 仓库路径以 byteboy 目录为基准；每次执行先确认 cwd、分支、工作区改动和实际依赖配置，不覆盖既有工作。
- 本次执行已按阶段实施代码并运行自动化检查；不提交、不推送、不发布。
- 后续实施逐阶段完成并记录真实命令、结果、失败原因及人工验证；未执行的检查不能标为通过。
- 不引入子代理或额外工作流依赖作为本计划的执行前提。
- 顺序：P0 → P1 → P2 → P3 → P4 → P5 → P6 → P7。P0 建基线，P1/P2/P3 优先处理正确性与发布可复现性；P7 为可选。
- 每阶段单独提交，避免把菜单接口、主题状态、动画调度改在同一个不可拆回滚的变更中。

## P0：建立可复现基线

文件：新增 `dozer/docs/analysis/2026-10-31-byteui-optimization-validation.md`；读取四仓 Cargo、CI、theme、menu、animation 相关文件。

- [x] 记录四仓 SHA、dirty 状态、Rust/系统版本、实际 tag、lockfile 来源及本地 patch；区分发布依赖与本地依赖。
- [x] 核对现有 shared-deps、host-deps、consumer-check 守卫，复用已有检查，列出本计划增加的覆盖。
- [x] 列出菜单生产调用、窗口开关及选择消息回传路径；列出主题 setter 与网页更新路径。
- [x] 保存外壳计算样式基线；已有不一致单独记录并确定目标。
- [ ] 记录 0/1/4 个动画可见、隐藏、最小化时的测量方法与数据，包含构建模式、窗口大小、缩放及采样时长。
  - 方法已写入基线文档 §6；真实 CPU/帧时间数据尚未采集。

出口：基线文件可供后续前后比较，事实与推断分开。

## P1：缩放校验与初始化

主要文件：`byteui/src/theme/icon_size.rs`、相应测试与 CHANGELOG；必要时调整两产品初始化调用。

- [x] 新增纯校验函数，正数钳制，非有限/非正拒绝；所有入口复用。
- [x] 覆盖主旧环境变量优先级、非法主变量回退旧变量、持久化回退、上下限及 NaN/Infinity。
- [x] `set_scale` 无效输入保持原值；测试 `zoom_by` 非法乘数和运算溢出。
- [x] `init_scale` 物化有效值，正常启动后不重复读环境；保留未初始化兼容路径。
- [x] 明确 reset、运行时 `set_theme` 与启动基准的关系，补测试，保留存盘格式。
- [x] 运行 ByteUI 定向测试与 `cargo test --locked`，记录结果。
  - 已运行 `cargo test --locked`（130 passed，含 doc-test）；`cargo fmt --check` 与 `cargo clippy --locked --all-targets -- -D warnings` 通过。

出口：BUI-01。回滚：恢复单模块实现，不牵连菜单/主题改造。

## P2：菜单窗口归属

主要文件：`byteui/src/native_menu.rs`；`bytehost` 窗口平台集成；`dozer/crates/dozer-app/src/platform/*_overlay.rs` 与菜单调用；`digger/crates/desktop/src/product.rs`、`adapters/*` 中菜单路径及 gallery。

- [x] 用最小原型验证从 `window::run` 内借用句柄执行菜单并将选择结果送回产品 update 的完整链路；检查 MainThread 与 Message 约束。
  - Digger 用 `window::run` + `Task` 回投；Dozer 的 winit 主循环在 App 上保存 `Weak<Window>`，事件处理前更新。
- [x] 落实显式窗口接口；若需注册表，先确定身份、代次、注销和迟到请求处理。
  - `show_for_window` / `show_align_no_icon_left_for_window` 已迁移全部生产调用；Dozer 用弱引用，浮窗销毁后 `upgrade()` 失败即返回无选择。
- [x] 迁移 Dozer 主窗及全部浮窗，删除关闭浮窗时恢复主窗的菜单接线。
- [x] 迁移 Digger 菜单与示例；确保菜单坐标始终来自同一个窗口。
- [x] 搜索确认生产路径不再依赖旧全局指针；最终删除不安全兼容实现。
- [ ] 自动测试纯路由/身份失效逻辑；人工验证主窗、两浮窗、关闭重开、快速切换、取消和选择菜单。使用平台内存诊断工具验证销毁路径，工具限制如实记录。
  - 已覆盖无 AppKit 句柄/空菜单返回 `None`；真实窗口与销毁路径未人工验证。
- [ ] 对受影响 crate 定向构建，并验证全部菜单操作结果，没有因异步迁移丢失选择消息。
  - `dozer-app --tests` 与 `digger-desktop --tests` 定向构建通过，`menu`/`native_menu` 相关单测通过；真实交互未验证。

出口：BUI-02。回滚：成组恢复 ByteUI API 与所有消费者，不只恢复函数签名。

## P3：发布组合与候选组合门禁

主要文件：`dozer/Cargo.toml`、`crates/*/Cargo.toml`、两产品 `.github/workflows/consumer-check.yml`、现有依赖守卫、`byteui/.github/workflows/ci.yml` 及候选集成 workflow。

- [x] Dozer ByteUI tag 上收 workspace，消费 crate 使用 workspace 依赖，保留所需 feature。
- [x] Digger 构建改为 locked；检查其锁文件是否已跟踪及可在无 patch 场景解析，若有问题先修锁文件来源。
  - 锁文件来源已修复；Dozer 库快照已升级到 `libs-2026-10-30a`（bytehost v0.4.20），无 patch 解析后无重复依赖，见基线 §1.3。
- [x] 共享依赖守卫覆盖 ByteUI source/ref 与 iced 版本；从实际 metadata/lockfile 验证，不能只比较清单字符串。
- [x] Dozer 外壳检查固定 ByteUI ref；CI 缺源码时报错，本地跳过需显式选项。
  - P4 合并共享外壳后该比对已退役（脚本与 CI 步骤删除），由委托 + 样式测试替代。
- [x] 定义候选 workflow 输入：四仓精确 SHA/固定 ref；在临时配置中 patch 对应源，生成临时锁文件后执行 locked 检查，不改用户全局 Cargo 配置。
- [ ] 确定跨仓 checkout 凭据与私有源读取能力；缺失时报出具体依赖，不能给假绿灯。
  - workflow 依赖 Actions checkout 的既有凭据；尚未在远端实际运行验证。
- [ ] 从干净 checkout 验证两产品发布组合；再验证候选组合。保存解析图并故意制造接口不兼容，确认候选门禁失败。
  - Dozer 干净 locked 解析通过；Digger 依赖图已无重复且 locked 元数据通过，但当前源码需要未发布的 ByteUI API，待 ByteUI tag 后做最终编译。候选门禁尚未触发。
- [x] 完成后清除临时 patch 并检查工作区，保留版本/命令/日志证据。
  - 本地临时 patch 已还原；`dozer`/`digger` 的 `.cargo/config.toml` 保持原样。

出口：BUI-03/BUI-04。后续阶段继续运行此门禁。

## P4：共享外壳配置收敛

主要文件：`byteui/src/theme/region.rs`、`dozer/crates/dozer-app/src/theme/region.rs`、`assets/theme/workspace.json`、外壳检查脚本/CI、旧拆库说明。

- [x] 按导出逐项区分公共区域与产品面板，确认 JSON 初始化没有重新覆盖共享定义。
- [x] 先增加最终样式对照验证，覆盖深浅方案和 scale 0.5/1/2/3 的 background、border、padding、gap、margin。
  - 自动化逐项对照覆盖深浅两套方案；scale 0.5/1/2/3 的数值由 ByteUI `region` 单测锁定，委托层另有“已缩放值不二次缩放”测试。截图级人工对照仍未执行。
- [x] Dozer 公共 accessor 委托 ByteUI，保留产品 accessor；有意产品差异用显式 override，先说明再保留。
- [x] 删除 JSON 重复项与已无意义的比对脚本/CI 步骤；确认独立 checkout 不再要求 sibling 源码做旧比较。
- [ ] 修订拆库 spec 的 region 边界；历史未尽事项加状态说明与本 spec 链接，保留历史上下文。
- [ ] 两产品进行主题/缩放 UI 检查，原生 WebView 矩形与面板头边界无偏移；运行候选组合门禁。

出口：BUI-05。回滚需同时恢复 JSON、委托、守卫及文档，不留下半套数据源。

## P5：动画限帧与暂停

主要文件：`byteui/src/feedback/math_curve.rs`、gallery、两产品使用动画的 view 与窗口状态适配。

- [x] 实施前查本地 iced 源码确认定时重绘及 Canvas update 行为；确定具体 API，不在计划中假定不存在的调用。
  - 已核对 `iced_core` 的 `RedrawRequest::At` 与 `Action::request_redraw_at`。
- [x] 将自驱动重绘改为下个 30 FPS 期限；增加可测调度逻辑，验证暂停、恢复和迟到事件。
- [ ] 提供宿主暂停参数，连接真实面板可见性与窗口状态；冻结暂停相位，恢复不追赶积压帧。
  - `view_animated_paused` 已提供并测试；产品面板尚未按可见性/窗口状态传参。
- [ ] 重测 P0 场景。仅在路径生成仍占主要开销时增加有界采样缓存，并测试尺寸/颜色/曲线/detail 变化的失效条件。
- [ ] 在常规及高刷新率显示条件下验证调度；记录实际重绘数、CPU 与可取得的帧时间。若收益不明显，保留实测结论，不增加无依据的缓存复杂度。
- [ ] 人工检查动画轨迹、颜色、缩放与多个实例相位，运行 ByteUI 测试及候选门禁。
  - ByteUI 单测已通过；人工与候选门禁未执行。

出口：BUI-06。回滚为调度/缓存独立变更，不影响其他组件。

## P6：主题快照与传播

主要文件：`byteui/src/theme/color.rs`；ByteHost 共享设置/窗口通知适配；两产品主题入口、WebView 主题与测试支持代码。

- [x] 完成 setter 盘点，明确自定义配色来源与兼容明暗提示的数据模型；覆盖 Dozer JSON 初始化而不覆盖用户选择。
- [x] 单锁保存 colors、scheme 元数据及 revision；旧 accessor 转发，任意 palette 不冒充命名配色。
- [x] 用纯 snapshot 测试验证更新一致性；必须修改全局状态的测试共用恢复 guard 和串行设施，清理模块私锁造成的假互斥。
- [x] 接入宿主主题变更流程，对现存窗口请求更新；逐项核对已有逻辑，避免重复发送通知。
  - 两产品原有主题切换链路保留并改用同一快照；未新增重复通知。
- [ ] 第一方网页从同一 snapshot 生成 CSS/方案，revision 拒绝过期更新；新开、reload、导航完成时初始化。第三方页面不注入。
  - CSS/方案已从同一快照生成；异步投递的 revision 过期丢弃尚未接入（现有投递为同步重建）。
- [ ] 验证主窗、设置窗、其他浮窗、审阅/Usage/Todo 等实际第一方页面及终端明暗提示；快速切换与迟到消息不回退主题。
- [ ] 验证保存失败仍保留会话选择并提示可重试；运行相关产品测试与候选门禁。

出口：BUI-07。不扩展为本轮统一所有 token 或重写组件参数。

## P7：可选组合组件与收尾

主要文件：两产品及共享面板中的面板头调用点；必要时新增 `byteui/layout` 下的组合组件与 gallery 示例。

- [x] 对 Files/Project/Todo/Usage 面板头做参数与布局对照，找到两个真实同构消费者才抽取。
- [x] 最小接口支持图标、标题、尾部操作与统一高度；不同布局继续保留在产品。
  - 新增 `byteui::layout::panel_heading::PanelHeading`，已迁移 Dozer `home_panel_head_with_actions`、Digger conversations/group-chat 适配层；分割线仍由产品保留。
- [ ] 验证两个消费者的布局、缩放及禁用操作；若不适合抽取，写明差异并将本项标记为“已评估，跳过”。
- [x] 汇总 BUI-01 至 BUI-08 证据及未解决问题，更新 spec 状态与历史剩余事项。
  - 见基线文档 §7；spec 状态已更新。
- [ ] ByteUI 执行 `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked`；受影响 ByteHost crate 执行定向测试。两产品执行 locked workspace/all-targets 构建及现有边界守卫，产品全量测试若有既有失败须单独列明。
  - 已执行：ByteUI `cargo test --locked --lib`；Dozer `cargo check -p dozer-app --tests`、`cargo test -p dozer-app theme::region`；Digger `cargo check -p digger-desktop --tests`（临时 byteui patch）。fmt/clippy/全量未执行。
- [ ] 编写迁移说明、破坏性 API 清单及完整回滚组合；确认无需发布的文档/内部变更可独立合入。

出口：BUI-08 与全部主线验收完成；未完成项明确保留，不能以“构建通过”替代生命周期、主题及性能验收。

## 后续发布检查点

- [ ] 候选精确 SHA 与完整 tag/lockfile 组合就绪，发布说明可审阅。
- [ ] 按 ByteUI → 必需的 ByteHost → 必需的 Dozer 库快照 → 两产品顺序准备升级，检查 Digger 间接 ByteUI 图没有旧源残留。
- [ ] 按届时用户授权执行发布与同日升级；本次文档编写不执行这些操作。

最终验证记录至少包含：阶段、仓库 SHA、实际依赖源、命令及退出状态、人工场景、性能前后数据、剩余限制与回滚组合。
