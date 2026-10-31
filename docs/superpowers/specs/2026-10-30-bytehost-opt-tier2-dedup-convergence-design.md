# bytehost 优化 Tier2：去重与收敛 规格

> 日期：2026-10-30。状态：草案，尚未实施。实施计划见 [plan](../plans/2026-10-30-bytehost-opt-tier2-dedup-convergence.md)。
> 本梯队聚焦消除 dozer/digger 两侧的重复代码与样板，行为等价是硬约束。

## 1. 目标与边界

把分析中识别的 6 处重复/样板收敛为单一实现：dozer 侧 3 处（overlay diff-webview、SurfaceExecutor 单槽包装、overlay 宿主外壳），digger 侧 3 处（双订阅实现、port.rs 超前建设、boot 样板）。

**不在本梯队**：bytehost 内部结构重构（Tier3）、性能（Tier4）、治理（Tier5）。去重不改变任何外部行为。

## 2. 已核对的代码基线

| 位置 | 当前事实 | 收敛责任 |
| --- | --- | --- |
| `dozer-app/src/platform/file_history_overlay.rs:263-296` 与 `edit_history_overlay.rs:279-311` | wry diff-webview 创建代码几乎逐字相同（仅变量名/消息名差异）；绕过 bytehost 自建 wry，与 browser_native.rs"不复刻 wry 初始化"原则矛盾 | dozer：抽公共 helper 或改走 bytehost web_child |
| `dozer-panel-usage/src/surface.rs`（~178 行）与 `dozer-app/src/extensions/codehealth/surface.rs`（~295 行） | 同构的 SurfaceExecutor 固定单槽包装 | dozer：上收至 dozer-panel-host |
| dozer ~15 个 overlay 宿主（各 200-500 行） | FocusTracker/cursor/modifiers 处理重复；机制已上收 bytehost 但外壳未模板化 | dozer：抽宏或 trait |
| `digger/crates/app-host/src/subscription.rs`（仅 tests/embedded.rs:28 引用）与 `effects.rs:664-697`（生产路径） | Changed/Resync/退避规则两份编码，易漂移 | digger：合并为一份 |
| `digger/crates/app-host/src/port.rs`（PortResetPlan/Stage） | 无生产消费者（设置未注册 Advanced 页） | digger：移除或标记延后 |
| `digger/crates/desktop/src/product.rs:3992-4067` + `:3952-3967` | 六段同型 take_receiver→Task::stream(unfold)、8 连发 attach_handle | digger：抽 boot helper |

## 3. 方案

### 3.1 dozer overlay diff-webview 去重

两份代码的差异仅在变量名与消息名。抽 `create_diff_webview(config: DiffWebviewConfig) -> wry::WebView` 公共 helper，config 携带标题/消息通道名/初始 HTML。若 bytehost web_child 能覆盖此场景则优先改走 web_child（需核对 web_child 是否支持 diff 类一次性 webview）；否则 helper 放 dozer-app 内部 platform 模块。

### 3.2 SurfaceExecutor 单槽包装上收

usage 与 codehealth 的 surface.rs 是同构固定单槽包装。抽到 dozer-panel-host（已共享 PanelIo/NoticeOutbox），提供 `SingleSlotSurfaceExecutor` 泛型或 trait 默认实现，两个消费方改为引用。

### 3.3 overlay 宿主模板化

~15 个 overlay 宿主的 FocusTracker/cursor/modifiers 样板抽为 `overlay_host!` 宏或 `OverlayHost` trait + 默认方法。机制层已在 bytehost，此处只模板化产品外壳。首个试点选 2-3 个最相似的 overlay，验证模式后再推广。

### 3.4 digger 双订阅合并

subscription.rs 仅测试引用，生产走 effects.rs 内联循环。把 Changed/Resync/退避规则统一到 effects.rs 的实现，subscription.rs 改为 re-export 或删除；测试改测生产路径。

### 3.5 digger port.rs 处置

PortResetPlan/Stage 无生产消费者。若 Advanced 页短期内不会注册则删除；若有明确路线图则加 `#[allow(dead_code)]` + TODO 注明预期接入时间。默认倾向删除（YAGNI）。

### 3.6 digger boot helper 抽取

product.rs 六段同型 take_receiver→Task::stream(unfold) 与 8 连发 attach_handle 抽为 `fn boot_receiver_stream<T>(...) -> Task<T>` 与 `fn attach_all_handles(shell: &mut ProductShell, ...)` helper。

## 4. 兼容与发布

- dozer 侧（3.1-3.3）随 dozer 正常发版，不涉及 bytehost。
- digger 侧（3.4-3.6）随 digger 正常发版。
- 3.1 若改走 bytehost web_child 则涉及消费方式变化，需核对 web_child 能力边界；若不匹配则退回内部 helper，不阻塞。
- 所有去重必须行为等价：测试先行，抽取前后功能不变。

## 5. 验收与完成定义

| ID | 场景 | 必须观察到的结果 |
| --- | --- | --- |
| T2-06 | file/edit history overlay 打开 diff | 行为与去重前逐像素一致；仅一份 wry 创建代码 |
| T2-07 | usage 与 codehealth 面板 Surface 执行 | 行为不变；surface.rs 仅存在于 dozer-panel-host |
| T2-08 | 任一 overlay 宿主打开/关闭/焦点 | 行为不变；样板行数下降 ≥50%（试点 overlay） |
| T2-09 | digger app 订阅 Changed/Resync/退避 | 行为不变；仅一份编码；测试覆盖生产路径 |
| T2-10 | digger 编译 + 测试 | port.rs 已删或标注；无 dead_code 警告残留 |
| T2-11 | digger desktop 启动 | boot 行为不变；product.rs 同型段落收敛为 helper 调用 |
