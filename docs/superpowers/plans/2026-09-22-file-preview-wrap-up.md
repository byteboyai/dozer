# 文件预览重构 —— 收尾 Implementation Plan

**Goal:** 收掉 Phase A–D 遗留的实现缺口,使总计划的「完成定义」全部成立:所有
文件类型都有内部 viewer / 可解释降级 / 外部打开,查看·搜索·Agent 在支持后端上
一致,资源受全局预算约束,恢复不丢数据。

**Spec / 总计划:** `docs/superpowers/specs/2026-09-22-file-preview-architecture-redesign.md`、
`docs/superpowers/plans/2026-09-22-file-preview-architecture-redesign.md`。

**人工验收:** `docs/superpowers/analysis/file-preview-acceptance-checklist.md`
(本计划每个 Task 的“真机验证”引用该清单对应小节)。

**原则:** 每步保持可构建、可回退;旧路径只在替代能力通过验收后删除;删除类
改动必须附功能对照测试与回退提交点。

**背景(现状):** CodeMirror / `vanilla-jsoneditor` 已常开;老 iced `CodeView`、
自研普通 JSON Tree、`syntect` 依赖均已删除(`086bb38d`、`5b8190b9`)。因此本计划
不含“把 feature 转默认”这类前置;`phase-{b,c,d}-progress.md` 里仍标未完成的
条目以本计划为准(部分已在上述提交完成)。

---

## T1. External/Unsupported 正式 fallback 页(替掉 Flyfish 兜底)

- [ ] `PreviewBackend::hosts_webview()` 对 `External` / `Unsupported` 返回
      `false`(`preview/backend.rs`),删除对应 adapter 兜底注释。
- [ ] 渲染层新增可解释 fallback 页:文件类型 / 原因(reason)+ 动作按钮:
      「在系统应用中打开」(`Message::PreviewOpenExternal`)、「以纯文本只读查看」
      (降级为 `Plain`/Windowed)、必要时「重试」。
- [ ] 复用现有 Failed 态的「在系统应用中打开」动作,不要另建一套系统打开逻辑;
      本任务补齐的是 External/Unsupported 的正式页面、reason、重试和安全纯文本退路。
- [ ] 压缩包 / 未知二进制 / 加密 / 损坏,以及**没有可用内部安全退路**的资源拒绝
      落此页,不得空白。普通文本超预算仍优先进入 Windowed,不能一律外部打开。
- [ ] 验收:清单 §2(`archive.zip` / `broken.zip` 与未知文件行)、§7 失败降级。
- [ ] 回退:保留 `hosts_webview()` 旧返回的提交点。

## T2. 清理 `PreviewTab` 旧 adapter 字段与一致性断言

- [ ] 逐步移除 `PreviewTab` 的 `tabular` / `json_tree` / `loading` / `truncated`
      / `loaded_bytes` / `total_bytes` 旧组合字段的消费,改为读 `backend` /
      `backend_state` / viewer 句柄(`preview/state.rs`)。
- [ ] 删除 `debug_assert_backend_consistent` 及所有调用(`preview/state.rs`、
      `preview/view.rs`、`workspace/view.rs`)。
- [ ] 保持 `state.rs` 测试与路由矩阵全绿;每删一个字段跑一次
      `cargo test -p dozer-app`。
- [ ] 验收:backend 成为唯一真相源;无 adapter 一致性断言残留。
- [ ] 回退:按字段分批提交,任一字段可单独 revert。

## T3. Agent 写操作下行接线(reveal / select / replace)

- [ ] 现状:`App::preview_editor_reveal/select/replace`(`app/app.rs:1027/1053/1081`)
      已实现但**无调用方**(`#[allow(dead_code)]`)。
- [ ] 设计 daemon/MCP → app 的下行命令通道(当前只有 app → dozerd 的
      `update_preview_context` 单向推送);`dozer-core::protocol` 增命令类型,
      `dozerd` 路由到目标 app(需 app 侧订阅/长连接),`dozer-mcp` 暴露工具。
- [ ] `replace` 必须 `expected_revision == tab.web_revision`,失配拒绝;目标在
      折叠区自动展开;目标 tab 若 Suspended,先 `load_preview_tab` 唤醒后再排队。
- [ ] 验收:清单 §6;daemon round-trip 测试;revision 冲突测试。
- [ ] 回退:命令通道 feature/开关隔离,关闭即回到只读 context。

## T4. 脏 tab 外部变更的“选择”动作

- [ ] 现有:脏 tab 外部变更只置 `web_error` 冲突提示。新增选择动作:
      「保留我的修改」/「重载磁盘(丢弃我的)」。
- [ ] 复用 `preview/recovery.rs` 快照与 `reload_nonce` 机制;选择后更新 backend
      状态与 dirty。
- [ ] 验收:清单 §3 最后一条;`preview` 单元测试覆盖两条分支。
- [ ] 回退:仅新增分支,不动现有冲突提示路径。

## T5. HTML 隔离 host 与 Flyfish 收敛

- [ ] HTML/HTM 不再直接 `file://` 加载(`preview/webview.rs:63`);走隔离 scheme
      (`dozer://html/` 或 sandboxed host),**禁止获得 editor JSON IPC**。
- [ ] Flyfish 零散 IPC(title 上报、⌘F 搜索注入等)迁移到 Phase 3 通用
      `WebviewEnvelope`,不并存第二套约定(`preview/webview_protocol.rs`)。
- [ ] Flyfish 接入 `ResourceManager` 的 reserve / suspend / destroy,及 Failed
      fallback 到 T1 的页面(`preview/resources.rs`、`runtime.rs`)。
- [ ] 验收:离线打开 HTML 正常;不可信页面无 editor IPC;资源诊断显示 Flyfish 登记。
- [ ] 回退:保留 `file://` 路径的提交点。

## T6. Streamed JSON 运行时收敛

- [ ] 现状澄清:JSONL/NDJSON 已路由为 `PreviewKind::Streamed` 并持有
      `PreviewBackend::Streamed` / `BackendState`;未完成的是运行时仍借用旧
      `json_tree` adapter/viewer。
- [ ] 移除 streamed 路径对旧 `json_tree` adapter/viewer 的依赖,使 viewer 句柄、
      Suspended/Loading/Ready/Failed 生命周期和资源 reserve/release 全部由统一
      backend 管理;超大 JSON 也按同一策略接入。
- [ ] 接 Rust 流式搜索(`preview/large_text.rs`)、稀疏定位、Agent 上下文
      (path/行号可选)。
- [ ] 坏行局部报错,不让整文件打不开;超大单行不全量建 DOM。
- [ ] 验收:清单 §2 `rows.jsonl`;与现有大 fixture 的首屏 / 内存对照。
- [ ] 回退:保留 `json_tree` streamed 旧路径的可切提交点,直到新运行时通过首屏、
      坏行、搜索、跳转和内存对照验收。

## T7. Tabular Agent context 补全(reveal cell/range)

- [ ] `dozer-core::protocol::PreviewTabularContext` 增“选中单元格 / 可见行列”
      与 reveal 命令;app 侧实现 reveal cell/range(滚动 + 选中)。
- [ ] XLSX 保持专用 grid;不改为 DOM 全量表格。
- [ ] 验收:清单 §6;`tabular` 与 protocol round-trip 测试。
- [ ] 回退:字段带 serde 默认值,旧客户端可解码。

## T8. 路由补全(文件名规则与未知 UTF-8)

- [ ] `Makefile` / `Dockerfile` / `LICENSE` / dotfile 按**文件名规则**优先于
      extension fallback 路由到 `Code`(`preview/router.rs`;Phase A Task 4 遗留)。
- [ ] 未知 UTF-8 文本路由到 `Code`(不再 Flyfish 兜底);未知二进制走 T1。
- [ ] SVG:图像渲染 + 可选 CodeMirror(XML)源码切换。
- [ ] 验收:清单 §2(`README_NO_EXT`、`Makefile`、`Dockerfile`、`LICENSE`、
      `.env`、`unknown.binblob`、`icon.svg`);router 表驱动测试逐项。
- [ ] 回退:router 表格化,yaml 开关可控。

## T9. 超大 / 非法 UTF-8 的安全退路

- [ ] 合法 UTF-16、非法 UTF-8、超大非法 UTF-8 / 二进制伪装文本分别覆盖;
      对无法无损解码者退 byte-safe streamed viewer 或外部打开,
      不进入会丢字节的 `fetch().text()` 路径(`preview/file_policy.rs`、
      `large_text.rs`、T1 页面)。
- [ ] 只读不等于字节安全:验收必须检查替换字符/乱码,并确保任何有损展示都不能
      被保存回原文件。
- [ ] 验收:清单 §2 `utf16le.rs` / `non_utf8.rs` / `invalid_utf8.rs` /
      `binary_spoof.rs`、§7 失败降级。
- [ ] 回退:保留“强制只读 CodeMirror”的提交点。

## T10. 滚动锚点精确还原

- [ ] 把持久化的逻辑 scroll anchor 更精确地还原为像素/行内偏移(现在只有
      `top_line`、`pending_view`);见 `preview/recovery.rs`、`preview_state.rs`。
- [ ] 验收:重启后滚动位置贴近关闭前。
- [ ] 回退:保留仅 `top_line` 的行为。

## T11. 文档 / 依赖 / 许可证 / 打包

- [ ] 旧 JSON spec(`docs/superpowers/plans/2026-09-19-json-preview-viewer.md`)状态
      更新:哪些结论被 `vanilla-jsoneditor` 替换、哪些流式算法保留。
- [ ] `CLAUDE.md` 关键约束同步(iced editor 删除、预览偏好渲染等)。
- [ ] 打包脚本 `scripts/build-macos-app.sh` 资源目录(editor/json-editor/flyfish)
      与许可证清单核对;`cargo udeps`/人工核对无残留直接依赖。
- [ ] 验收:release 打包 smoke test,无 404 / 无 CDN。

## T12. 最终路由矩阵与全量验收

- [ ] 自动化:路由矩阵每项 + 大小 / 能力组合(Phase A fixture 生成器)。
- [ ] `cargo test -p dozer-app`、`cargo test --workspace`、
      `cargo clippy --all-targets`、`cargo fmt --check`;前端 `tsc`/`npm test`/build。
- [ ] 人工:按 `file-preview-acceptance-checklist.md` 全量跑一遍,逐项记录。
- [ ] 更新总计划「完成定义」勾选与各 phase 进度文档。

---

## 顺序建议

1. T1(独立、消除空白兜底,风险低,先做)。
2. T2(纯内部收敛,分批小步)。
3. T8 / T9(路由与安全退路,与 T1 衔接)。
4. T4、T7(体验补全)。
5. T5、T6(隔离与统一后端,改动面大)。
6. T3(需协议/daemon/MCP 协同,单独设计通道)。
7. T10、T11、T12(收尾与非功能项)。

## 完成定义(对齐总计划)

全部成立才算重构完成:①老 iced editor 与重复普通 JSON tree 已退役;②所有文件
类型有内部 viewer / 可解释降级 / 外部打开;③查看·搜索·Agent 在支持后端一致;
④单文件与跨项目资源受全局预算;⑤启动恢复不随历史 tab 线性加载;⑥脏内容 /
外部变更 / revision 冲突不静默丢数据;⑦大文件降级后仍可打开、搜索、跳转。
