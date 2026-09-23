# Agent 下行预览命令通道设计(T13a)

> 状态(2026-09-24):协议层已落地(`dozer-core::protocol`),app 侧纯逻辑
> (`PreviewPane::apply_preview_command`)已落地并单测;daemon→app 的传输接线
> 与排队/取消仍是后续工作。本文件是 T13a 的协议 spec。

## 1. 目标与边界

让 agent(经 `dozer-mcp`)能对用户正在看的**预览 tab** 做只读导航(reveal /
select / reveal_cell),并在 revision 匹配时做受守卫的写入(replace)。本通道
**不**绕过 Dozer 的核心原则:预览默认是"查看",写入是显式、可校验的例外。

- 只读导航不与写权限阻塞:reveal/select/reveal_cell 只看 backend 是否支持。
- replace 只在**可写 CodeMirror**上、且 `expected_revision` 与 app 当前一致时
  接受;绝不猜 revision 增量。
- 命令必须有**确定终态**,不允许无限等待。

## 2. 传输拓扑(已实现:poll 式)

```
dozer-mcp ── Request::RunPreviewCommand ──▶ dozerd ──(队列 + waiter)──▶ 回 Reply
dozer-app ── Request::TakePendingPreviewCommands(project) ──▶ dozerd 取走队列
dozer-app ── Request::ReportPreviewCommandOutcome ──▶ dozerd 投递给 waiter
```

- app 与 dozerd 仍是**短连接 request/reply**(每次调用新建连接);不引入常驻控制
  连接。app 侧由后台轮询任务(约 400ms,只在聚焦项目存在时请求)取待处理命令。
- 多 app 实例:目前一个 dozerd 服务一个 GUI;按 `project_id` 分队列。
- app 不在线 / 超时:dozerd 在 `RunPreviewCommand` 上 `timeout(PREVIEW_COMMAND_TIMEOUT_MS)`
  后回 `PreviewCommandOutcome::Timeout`,并 `forget` 掉队列里同 id 的命令。
- app 退出/tab 关闭:命令在 app 侧定位不到 tab → `NotFound`;超时 → `Timeout`。

## 3. 消息形状(已实现)

- `Request::RunPreviewCommand { command: PreviewCommand }`
- `Reply::PreviewCommandResult { outcome: PreviewCommandOutcome }`
- `PreviewCommand { request_id, project_id, target, action, expected_revision? }`
  - `target`: `Tab { panel, tab_id }` 或 `Path { path }`。
  - `action`: `Reveal` / `Select` / `RevealCell` / `Replace`。
- `PreviewCommandOutcome`:
  - `Accepted { request_id }`
  - `NotFound { request_id, detail }`
  - `StaleRevision { request_id, current_revision }`
  - `UnsupportedBackend { request_id, detail }`
  - `LoadDenied { request_id, detail }`
  - `Timeout { request_id }`
  - `InternalError { request_id, detail }`

## 4. app 侧语义(已实现 `PreviewPane::apply_preview_command`)

- 目标 tab 不存在 → `NotFound`。
- `Reveal` / `Select`:仅 `uses_editor_host()` 的 tab;否则
  `UnsupportedBackend`。命中即排队 `RevealPosition` / `SelectRange` → `Accepted`。
- `RevealCell`:仅表格 tab;目标 sheet 未加载 → `LoadDenied`(由上层按 T3 规则
  reserve/物化后重放一次);否则写 scroll + selection → `Accepted`。
- `Replace`:只读 / 不可写 backend → `UnsupportedBackend`;缺
  `expected_revision` → `InternalError`;失配 → `StaleRevision { current_revision }`;
  匹配 → 排队 `ReplaceRange` → `Accepted`。

## 5. 排队、取消、超时(待接线)

- 同一 tab 的 in-flight 命令按到达顺序排队;`request_id` 唯一,重复 id 直接
  拒绝(`InternalError`)。
- suspended tab 收到导航:先按 T3 reserve → 物化 → 就绪,再执行一次性命令;
  期间命令可取消(tab 关闭/项目切换 → `NotFound`/`Timeout`)。
- 每个命令有超时(建议 2s);超时回 `Timeout`,不永久挂起。
- 折叠区目标:reveal/select 命中折叠范围时先展开最小包含范围(host 侧
  `unfoldEffect`),再定位——当前 host `restore_view_state` 已有 unfold 原语,
  命令路径待接入。

## 6. 版本与权限

- envelope 已带 `protocol_version`;不匹配直接拒绝(不回 outcome)。
- 权限与本机其它写工具同信任级(不额外鉴权);但 replace 的 revision 守卫是
  **强制**的,防止 agent 基于过期上下文覆盖用户改动。
