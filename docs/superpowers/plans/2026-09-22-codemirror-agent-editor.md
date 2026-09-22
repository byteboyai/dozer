# 文件预览重构 Phase B：CodeMirror 与 Agent Editor Implementation Plan

**Goal:** 将普通代码/文本迁移到 CodeMirror 6，建立安全、版本化的 Rust/WebView
协议，并完整替代老 iced editor 的查看、搜索、简单编辑和 Agent 操作能力。

**Depends on:** `2026-09-22-file-preview-foundation.md`。

**Spec/Master:** `../specs/2026-09-22-file-preview-architecture-redesign.md`、
`2026-09-22-file-preview-architecture-redesign.md`。

> 进度(2026-09-22):Task 1 主体、Task 3 主体、Task 2 的 scheme/CSP/host 描述
> 已落地。首个运行时垂直切片也已接通：`codemirror` feature 开启后 Code route
> 不再构造 iced CodeView，`desired_webviews` 会生成带 Rust 可信 binding 的 editor
> spec，runtime 创建 WebView、校验并派发 IPC，App 镜像 revision/selection/viewport/
> dirty 并处理保存；feature 关闭仍走原 iced 路径。Task 4–7 的完整生命周期、
> Agent/daemon 链路和最终迁移仍未完成，不能把本切片视作 Phase B 整体验收通过。

## Task 1：前端工程与离线产物

**Files:**

- Create: `crates/dozer-app/web/editor/`
- Create generated assets under: `crates/dozer-app/assets/editor/`
- Modify: build/package scripts and license inventory

- [ ] 固定 CodeMirror core、view、commands、search、language 和首批语言包版本。
- [ ] 配置 TypeScript + 单测 + production bundling；产物无 CDN/动态远端 import。
- [ ] 构建只输出确定性 JS/CSS/source map policy；检查产物不得含绝对本机路径。
- [ ] 建 ByteBoy2077 深浅主题、JetBrains Mono 与系统 CJK fallback。
- [ ] 实现基础 editor：line number、fold gutter、search/replace、selection、history、
  read-only；明确关闭补全/LSP/lint/multicursor/minimap。

## Task 2：安全 scheme 与 WebView host

**Files:**

- Modify: `crates/dozer-app/src/assets.rs`
- Create: `crates/dozer-app/src/preview/webview_host.rs`
- Create: `crates/dozer-app/src/preview/code_host.rs`
- Modify: `crates/dozer-app/src/runtime.rs`
- Modify: `crates/dozer-app/src/platform/window_events.rs`

- [ ] 新增 `dozer://editor/` 静态资源 namespace，拒绝 traversal 和任意文件读取。
- [ ] 设置严格 CSP：脚本/样式/字体仅 self，禁网络、frame、任意导航。
- [ ] WebView 绑定 project/tab/document/path；JS 消息不允许自报任意路径。
- [ ] 抽取 host pool key、预算登记、tab/pane 激活、resize、销毁公共契约。
- [ ] macOS 上下文菜单使用 NSMenu；模态弹窗复用 Settings 独立原生窗口，不增加
  临时隐藏 WebView 的 `set_visible` 逻辑。
- [ ] 测试无网络打开、资源 404、非法路径、tab id 碰撞、两个 pane 同时存在。

## Task 3：通用消息 envelope

**Files:**

- Create: `crates/dozer-app/src/preview/webview_protocol.rs`
- Frontend: editor protocol types/tests

```rust
pub struct WebviewEnvelope<T> {
    pub protocol_version: u32,
    pub project_id: i64,
    pub panel: PanelKind,
    pub tab_id: usize,
    pub document_id: String,
    pub revision: u64,
    pub request_id: Option<String>,
    pub payload: T,
}
```

- [ ] 明确 command/event tagged enum，未知版本/类型返回错误但不 panic。
- [ ] 校验当前 project/panel/tab/document/revision 后才能应用事件。
- [ ] 实现 request/response correlation 和 WebView 重建后的旧响应丢弃。
- [ ] selection/viewport 节流；document changes 为 CodeMirror change set，不逐键全文。
- [ ] 测试乱序、重复、过期 tab、revision 回退、错误 document id 和恶意超大消息。

## Task 4：文档生命周期与简单编辑

**Files:**

- Modify: `preview/backend.rs`, `preview/state.rs`, `preview/view.rs`
- Modify: workspace update/message/save paths

- [ ] 实现 `set_document/ready/document_changed/save_requested/failed`。
- [ ] CodeMirror loaded 时 document + revision 为编辑缓冲权威；Rust 保存时获取匹配
  revision 的 snapshot，保存完成只清对应 revision 的 dirty。
- [ ] 保持 LF/CRLF、BOM 和末尾换行；非 UTF-8 默认只读。
- [ ] 保存使用同目录临时文件 + flush/rename；失败保留 dirty 和 recovery 数据。
- [ ] 实现输入、删除、Enter/Tab、剪贴板、undo/redo、局部/全部替换和 ⌘S。
- [ ] 外部文件变化：clean 自动 reload，dirty 进入冲突状态并提供选择。

## Task 5：搜索、折叠与导航

- [ ] ⌘F/⌘R 使用 CodeMirror 内部 UI，移除 Code tab 对 iced Find 条的依赖。
- [ ] 搜索支持 query、case、next/prev、replace current/all，状态可被 Rust 查询。
- [ ] 折叠使用语言服务；未知语言可用缩进/括号 fallback 或无折叠并明确能力。
- [ ] 实现 `reveal_position/select_range`；目标在折叠区自动展开最小区域。
- [ ] 视图状态序列化 cursor/selection/top-line/fold ranges，像素值不持久化。
- [ ] 验收拖动滚动条到底后最后一行真实可见，行号与 wrap/fold 一致。

## Task 6：Agent/daemon/MCP 全链路

**Files:**

- Modify: `crates/dozer-core/src/protocol.rs`
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozerd/src/preview_context.rs`, `server.rs`, `ide_bridge.rs`
- Modify: `crates/dozer-mcp/src/server.rs`
- Modify: `crates/dozer-app/src/workspace/hook.rs`, `workspace/state.rs`

- [ ] 扩展 PreviewContext：revision、mode、read_only、cursor/selection、selected_text
  （有上限）、visible lines；旧字段/客户端可兼容解码。
- [ ] editor → app 与 app → dozerd 两段分别节流/合并；tab 切换/失焦/关闭立即 flush。
- [ ] MCP 读取的是 dozerd 最新快照，不直接依赖 WebView 句柄。
- [ ] 增加 reveal/select/replace 的 app 侧命令入口；replace 必须匹配 revision。
- [ ] Agent 操作休眠 tab 的排队语义留接口，Phase C 接入真实唤醒。
- [ ] 测试 daemon round-trip、MCP 输出、selection 大小上限和 revision 冲突。

## Task 7：路由切换与老 editor 兼容期

- [ ] 小范围 feature flag/开发开关先让源码进入 CodeMirror。
- [ ] 路由矩阵中的全部代码/文本迁移；Markdown/HTML source mode 暂可后置 Phase D。
- [ ] 旧 CodeView 只作为短期 fallback，不新增功能。
- [ ] 对比选择、搜索、保存、主题、缩放、IME、Agent context 全部通过后默认开启。
- [ ] 本阶段不删除 `code_editor/`；Phase C 大文件和恢复完成后由 Phase D 删除。

## Phase B 验收

- [ ] 行号、真实滚动条、折叠、搜索、简单编辑全部准确。
- [ ] 中文 IME、emoji、组合字符和坐标转换测试通过。
- [ ] Agent 可感知选区、跳转、选择；过期写入被拒绝。
- [ ] WebView 离线、安全、NSMenu/Settings 弹窗层级正常。
- [ ] Rust 与前端测试、cargo test/clippy/fmt、生产 bundle 通过。
- [ ] 回填 master plan Phase 3–4。
