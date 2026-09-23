# 文件预览架构重构——收尾 Implementation Plan（按当前代码重订）

**Goal:** 在不回退现有 CodeMirror、vanilla-jsoneditor、Tabular 和 Flyfish
能力的前提下，完成文件预览架构的运行时收敛：所有文件有可解释路由和安全退路，
viewer 状态与资源生命周期只有一份真相，恢复不丢数据，大文件始终有界读取，Agent
导航/写入具备可验证的端到端协议。

**设计依据:**

- `docs/superpowers/specs/2026-09-22-file-preview-architecture-redesign.md`
- `docs/superpowers/plans/2026-09-22-file-preview-architecture-redesign.md`
- `docs/superpowers/analysis/file-preview-acceptance-checklist.md`

本计划取代旧版 wrap-up 对 T2、T3、T5、T6、T10 的描述。各 phase progress
文档若与本计划的“当前事实”冲突，以代码和本计划为准。

---

## 0. 当前事实与不可误判项

开始实施前，以下事实必须保持准确；后续提交若改变它们，应同时更新本节和验收清单。

1. CodeMirror 和 vanilla-jsoneditor 已常开；老 iced `CodeView`、自研普通 JSON
   Tree、`syntect` 已删除。
2. `PreviewBackend` 当前是**路由描述**，`BackendState` 是不带 payload 的生命周期
   枚举；Tabular 等真实运行时对象尚未统一归入 backend-owned runtime。
3. `PreviewTab` 仍持有 `tabular/loading/truncated/loaded_bytes/total_bytes` 等迁移期
   字段；当前已经没有 `json_tree` 字段。
4. JSONL/NDJSON 当前路由为 `PreviewKind::Json + JsonMode::Text`，由 CodeMirror
   文本模式查看。代码中尚无 `PreviewKind::Streamed`、`PreviewBackend::Streamed`
   或 Streamed JSON viewer。
5. `ResourceManager` 已有预算和淘汰顺序算法，但运行时目前主要登记带
   `editor_binding` 的 host；Flyfish、Tabular 等没有完整纳入。淘汰 WebView 句柄
   也尚未等价于将 tab 状态迁移为 `Suspended`。
6. 启动恢复已有 Suspended 壳和按需物化；这不等于运行期资源淘汰闭环已经完成。
7. `App::preview_editor_reveal/select/replace` 有 app 内入口，但 daemon/MCP 没有
   下行命令通道。
8. 大文件已有 Rust 流式搜索、稀疏行索引和 Windowed CodeMirror。超长单行必须
   按固定字节预算读取；不得使用会先分配整行的 `read_until` 再裁剪。

## 1. 实施原则

- 每个 task 结束时必须可构建、可运行、可单独回退。
- 先建立新状态归属，再删除旧字段；禁止“先删 adapter，再临时找地方存状态”。
- 资源淘汰必须改变应用状态，不能只从 WebView pool 删除句柄。
- 非 UTF-8 的“只读”不等于字节安全；有损解码内容不得保存回原文件。
- 外部打开只打开当前 tab 绑定路径，不拼接脚本、不隐式执行文件。
- 所有 WebView 命令使用具名 envelope；不得新增任意 JS 命令入口。
- 大 fixture 的验收必须同时检查首屏、RSS 和读取上限，不能只检查最终能否显示。
- 删除旧路径前必须有功能对照测试和明确的回退提交点。

---

## T0. 校准文档与验收基线

**目的:** 先消除会把实现引向不存在结构的错误描述。

- [x] 修正人工验收清单：`rows.jsonl` 当前期望为 CodeMirror Text，而不是
      “已存在的 Streamed 树”；把 Streamed 树列为 T8 完成后的期望。
- [x] 删除所有“streamed 仍借用 json_tree adapter”的现状描述。
- [x] 将 `PreviewBackend`、`BackendState`、`PreviewTab` 当前字段快照写入 phase
      progress，避免后续再次根据过期设计推断代码。
- [x] 给每项人工验收增加“当前预期 / 目标预期”，迁移期间不把已知缺口误报为回归。

**验收:** 文档中不存在 `PreviewBackend::Streamed` 已实现、`PreviewTab::json_tree`
仍存在等错误断言。

---

## T1. External / Unsupported 正式 fallback 页面

**目的:** 任何无法安全内嵌的文件都显示明确页面，不再依赖 Flyfish 偶然兜底或空白。

- [x] `PreviewBackend::hosts_webview()` 对 `External` / `Unsupported` 返回 false。
- [x] iced 渲染层增加统一 fallback 页面，显示文件类型、路径摘要、route reason、
      failure reason 和安全提示。
- [x] 页面动作按能力显示：
  - “在系统应用中打开”：复用 `Message::PreviewOpenExternal`；
  - “以纯文本只读尝试”：只在内容探测允许时出现(`route.content_is_text`；
    二进制/压缩包不给)；
  - “重试”：仅对 retryable Failed 状态出现。
- [x] 压缩包、加密/损坏文件、未知二进制和无安全内部退路的资源进入该页面。
- [x] 普通文本仅因超预算时必须进入 Windowed，不得被统一送去 External。
- [x] Failed、External、Unsupported 使用同一套动作生成逻辑，避免三个页面漂移
      (`preview::fallback_actions`)。

**自动化:** router/backend/fallback action 表驱动测试；路径不存在、权限失败、损坏
压缩包不 panic。

**真机:** 验收清单 §2 的 `archive.zip`、`broken.zip`、`unknown.binblob`，以及 §7。

**回退点:** 保留 External/Unsupported 仍 host Flyfish 的上一提交。

---

## T2. 建立统一 PreviewRuntime 状态容器

**目的:** 在删除迁移字段之前，为真实 viewer 句柄和加载状态建立唯一归属。

- [ ] 设计并落地运行时容器，推荐形态：

  ```rust
  enum PreviewRuntime {
      None,
      Tabular(TabularState),
      Windowed(WindowedRuntime),
  }
  ```

  WebView 句柄仍由平台 pool 持有，但 tab 必须持有可判定其 desired/resident 状态的
  runtime metadata。也可采用 `BackendState<T>`，但不得同时保留两套权威状态。
- [ ] 明确每个 backend 的 `Suspended → Queued → Loading → Ready/Failed` 转换、
      runtime 创建/销毁点和错误承载位置。
- [ ] 将 Tabular 的 Loading/Ready/Error 数据迁入 runtime；渲染层不再直接猜
      `tabular.is_some()`。
- [ ] 将 Windowed 的 index、当前窗口范围、截断状态和加载错误归入 runtime。
- [ ] 定义 WebView backend 的 resident metadata：host 类型、estimated bytes、
      visible/active、document revision，不把真实 `wry::WebView` 塞进业务状态。
- [ ] backend 描述只回答“该用什么看”，runtime 只回答“当前加载到什么状态”；
      两者职责写入模块文档和测试。

**自动化:** 各 backend 的合法/非法状态迁移；Suspended 不产生 desired viewer；
Ready 缺 runtime 时 debug/test 失败；失败重试能重新进入 Loading。

**完成门槛:** T2 完成前不得开始删除 `PreviewTab::tabular/loading`。

---

## T3. ResourceManager 驱动真实 suspend / evict 闭环

**目的:** 达成原设计的跨项目总预算，而不是只维护一张与实际对象可能脱节的登记表。

- [ ] 为 CodeMirror、vanilla-jsoneditor、Flyfish、Tabular、Windowed 分别定义
      估算成本与是否占 heavy WebView 名额。
- [ ] 所有 viewer 使用统一 `(project_id, panel, tab_id)` 资源键，避免 Files 与
      Project 面板同 tab id 冲突。
- [ ] reserve、register、touch、dirty、has_recovery、saving、agent_writing、active
      在真实生命周期事件中更新。
- [ ] `NeedEviction` 通过 app 消息执行：
  1. 校验候选仍可淘汰；
  2. 请求序列化视图状态；
  3. 脏 tab 确认 recovery 已成功；
  4. 销毁 runtime/WebView；
  5. 将 backend state 迁为 Suspended；
  6. 最后 release 预算。
- [ ] 禁止在 `sync_webview_pool` 中只删除 pool 句柄却保持 tab Ready；否则下一帧会
      重新 desired，产生销毁/重建抖动。
- [ ] reserve denied 时显示可解释占位，不得静默不创建导致空白。
- [ ] 切换到 Suspended tab 时重新 reserve；成功后物化，失败时保持壳并显示原因。
- [ ] 增加资源诊断快照：各 viewer 成本、总预算、heavy 数、最后访问、不可淘汰原因。

**自动化:**

- 多项目淘汰顺序：后台干净 → 当前项目非活动干净 → 有 recovery 的脏 tab；
- active/saving/agent_writing/无 recovery 的脏 tab 不可淘汰；
- 连续多帧不反复销毁/重建同一 tab；
- Files/Project 相同本地 tab id 不冲突；
- Tabular/Flyfish 同样计入预算。

**真机:** 验收清单 §7，多项目各打开若干重型文件，观察诊断与 RSS。

---

## T4. 清理 PreviewTab 迁移字段

**前置:** T2、T3 完成。

- [ ] 按独立提交依次迁移并删除 `tabular`、`loading`、`truncated`、
      `loaded_bytes`、`total_bytes`。
- [ ] 清理已经过期的 iced editor/json_tree adapter 注释。
- [ ] 删除 `debug_assert_backend_consistent`，改为针对 backend/runtime/state 的结构化
      invariant 测试；不要因删除断言而失去一致性保护。
- [ ] `hosts_webview`、`uses_editor_host`、desired lists、渲染分派、保存、搜索、
      恢复全部只读取 backend + runtime + backend_state。
- [ ] 每删一个字段分别执行 dozer-app 全量测试，禁止一次性机械删除。

**完成门槛:** `PreviewTab` 不再含用来猜 viewer 类型的平行 Option；不存在创建期和
运行期两套 loading 真相。

---

## T5. 路由补全与安全内容探测

- [x] 文件名注册表优先于扩展名 fallback，但仍受内容安全检查约束：
  - `Dockerfile` → dockerfile；
  - `Makefile` / `GNUmakefile` → makefile；
  - `LICENSE*` / `NOTICE*` → plaintext；
  - `.gitignore` / `.dockerignore` → plaintext；
  - `.env` 及明确允许的变体 → plaintext/properties。
- [x] 不使用“所有 dotfile 都是文本”的宽泛规则；含 NUL/明显二进制内容必须走安全
      fallback。
- [x] 未知 UTF-8 文本路由到 Code；未知二进制路由到 T1 页面。
- [x] 空文件按可编辑纯文本处理，除非扩展名命中专用 viewer。
- [x] SVG 默认图像渲染，并提供 CodeMirror XML 源码模式。
- [ ] route reason 能区分文件名规则、扩展名规则、内容探测、预算降级和用户 mode。
      （文件名/扩展名/内容探测/用户 mode 已区分；预算降级由 `file_policy`/
      `windowed` 单独表达，未进入 `RouteReason`。）
- [x] 自动化:router 表覆盖验收清单 §2 所有 fixture，并加入文件名正确但内容含 NUL、
      大小写变体和双扩展名案例。

---

## T6. 非 UTF-8 与字节安全退路

- [x] 区分 UTF-8、带 BOM UTF-16LE/BE、非法 UTF-8、二进制伪装文本
      (`FileProfile` encoding/content_kind + `is_lossy_text`;route 带
      `encoding_lossy`/`encoding_utf16`)。
- [x] 合法 UTF-16 可解码只读展示；若暂不支持无损保存，UI 必须明确只读原因
      (editor URL `enc=utf16` → 顶部只读提示)。
- [x] 非法编码不得进入 `fetch().text()` 后再保存；提供 byte-safe 只读查看器或 T1
      外部打开。(采用:有损文本只读展示 + 保存恒拒绝;二进制伪装落 T1 页。)
- [x] 有损展示必须带明显提示，保存命令恒拒绝(`lossy=1` 顶部提示;
      `PreviewTab::can_save` 守卫 + host `saveHandler` 双重拒绝)。
- [x] 文件画像采样不能把“源码扩展名”凌驾于二进制检测结果之上(binary 判定提前)。

**自动化:** `utf16le.rs`、`non_utf8.rs`、`invalid_utf8.rs`、`binary_spoof.rs`；
断言原始文件 hash 在查看、搜索、误触 ⌘S 后不变。(路由/只读/`can_save` 断言已加;
保存拒绝由 `can_save` 单测覆盖,未做端到端 hash fixture。)

---

## T7. HTML 隔离 host

- [ ] HTML/HTM 不再直接加载 `file://`；新增 `dozer://html/` 隔离 host 或等价
      sandboxed 页面。
- [ ] HTML host 只允许读取当前绑定文件及明确允许的相对资源；禁止访问 editor/
      json-editor IPC。
- [ ] CSP 默认无网络、无任意脚本；如产品需要脚本预览，必须作为显式危险 mode，
      不纳入本次默认路径。
- [ ] Source 模式继续使用 CodeMirror，切换 mode 不同时驻留两套重型 viewer。

**自动化:** 路径穿越、跨文件读取、editor envelope 注入均失败；离线 HTML 正常。

**回退点:** 保留旧 `file://` 行为的独立提交，仅用于紧急回退。

---

## T8. 从零建立 Streamed JSON backend

**注意:** 这是新增能力，不是“移除旧 json_tree adapter”。

- [ ] 增加 `PreviewKind::Streamed`、`PreviewMode::Streamed`、
      `PreviewBackend::Streamed(StreamedBackend)` 及持久化宽容解析。
- [ ] JSONL/NDJSON 默认迁移到 Streamed；保留“原文文本”可选 mode，作为迁移回退。
- [ ] Rust 逐行解析，每行独立 root；坏行产生局部错误节点，不使整个文件失败。
- [ ] 展示层必须虚拟化，只持有可见窗口和有限展开节点；禁止整文件 DOM/JSON tree。
- [ ] 接入 T2 runtime 与 T3 资源生命周期。
- [ ] 接入 Rust 全文件搜索、稀疏行索引、结果跳转、全局行号和 Agent context。
- [ ] 超大单行 JSON 受字节预算限制，显示截断/解析失败说明，不全量分配 DOM。
- [ ] 普通超大 `.json` 根据画像和 JSON Tree 预算决定 Tree、Text/Windowed 或
      Streamed，不默认把任意巨型 JSON 送入 vanilla-jsoneditor。

**自动化:** 正常 JSONL、空行、坏行、CRLF、UTF-8、多 MB 单行、百万行、取消加载；
文本回退 mode round-trip。

**真机:** `rows.jsonl`、大 JSONL、坏行 JSONL 的首屏、搜索、跳转、RSS。

---

## T9. Flyfish IPC envelope 与资源收敛

- [ ] 为 Flyfish 建立稳定的 project/panel/tab/document binding。
- [ ] title、搜索、状态和失败事件迁移到通用 `WebviewEnvelope`；保留 Flyfish 自己的
      payload enum，不与 editor command 混用。
- [ ] 所有入站事件校验 protocol version、project、panel、tab、document。
- [ ] 接入 T3 资源 reserve/suspend/destroy；Failed 回落 T1 页面。
- [ ] 删除旧零散 JS 搜索注入前，完成新旧功能对照测试。

**范围控制:** T7 HTML 隔离与本任务分开提交；安全域和 IPC 迁移不可揉成一个不可
回退的大提交。

---

## T10. 脏 tab 外部变更冲突处理

- [x] 脏 tab 检测到磁盘变化后进入显式 Conflict 状态，不只写一条 `web_error`
      (`PreviewTab::conflict` + `mark_disk_conflict`)。
- [x] 提供“保留我的修改”和“重载磁盘（丢弃我的修改）”。丢弃动作需要二次确认
      (`arm_conflict_reload` → `discard_conflict_and_reload`)。
- [x] “保留”继续以 editor buffer/recovery 为权威；下一次保存覆盖前再次校验磁盘
      revision，避免提示后磁盘又变化(`PreviewTab::save_gate` 重新置冲突并拒绝)。
- [x] “重载”删除对应 recovery、清 dirty、增加 reload nonce，并重置 revision 基线。
- [ ] app 异常退出后若 recovery 与磁盘 revision 冲突，恢复时进入相同 Conflict UI。
      (未做;启动恢复仍是 recovery 优先,未与磁盘 mtime 对比。)

**自动化:** 干净自动重载、脏保留、脏丢弃、冲突后二次外部修改、恢复时冲突。
(干净自动重载/脏保留/脏丢弃/二次外部修改已有测试;恢复时冲突未做。)

---

## T11. 完整视图状态恢复

**目的:** 先闭合逻辑状态，再考虑像素级增强。

- [ ] `EditorEvent::ViewState` 完整保存 cursor、selection、top line、folds；当前不能
      只消费 selection。
- [ ] `PreviewTab` runtime mirror、`preview_state` 持久化和恢复命令字段一致。
- [ ] Suspended/evicted 前请求一次 `SerializeViewState`，设置超时；失败时至少保留
      最近一次节流镜像。
- [ ] 恢复顺序固定：set document/window → folds → selection/cursor → scroll。
- [ ] Windowed 只恢复全局行锚点，不持久化局部文档 offset。
- [ ] Tabular 恢复 sheet/row/column。
- [ ] 完成逻辑恢复后，再增加可选像素/行内偏移，使重启位置更贴近原视图；像素恢复
      不得成为 cursor/folds 基础闭环的前置。

**自动化:** 普通 CodeMirror、折叠区内 cursor、Windowed、Tabular、被资源淘汰后
重新物化五类 round-trip。

---

## T12. Tabular Agent context 与导航

- [ ] `PreviewTabularContext` 增加选中单元格/范围和可见行列；serde 字段带默认值。
- [ ] app 实现 reveal cell/range：先切 sheet，再滚动并选中。
- [ ] XLSX 保持原生虚拟化 grid，不转换为全量 DOM。
- [ ] suspended Tabular 收到导航命令时按 T3 规则先 reserve/物化，再执行一次性命令。

**自动化:** protocol round-trip、越界钳制、sheet 不存在、suspended 唤醒。

---

## T13. Agent 下行通道：先协议，再导航，最后写入

### T13a. 独立协议设计

- [ ] 先形成 daemon → app 命令通道小型 spec，明确：app 注册/重连、多 app 实例
      选择、project/tab 定位、request id、reply、超时、取消、权限和版本兼容。
- [ ] 命令结果必须区分 accepted、not found、stale revision、unsupported backend、
      load denied、timeout、internal error。
- [ ] app 退出或 tab 关闭时，待处理命令收到确定失败，不能无限等待。

### T13b. 只读导航

- [ ] 先暴露 reveal/select；它们是导航操作，不与 replace 一起被写权限阻塞。
- [ ] suspended tab：reserve → load → ready → 执行；期间同 tab 命令按 request id
      排队并可取消。
- [ ] 折叠区目标先展开最小包含范围。

### T13c. revision-guarded replace

- [ ] replace 仅支持可写 CodeMirror，必须携带 `expected_revision`。
- [ ] app 执行前再次比较当前 revision；失配拒绝并返回 current revision。
- [ ] 成功返回新 revision 和实际替换范围；不得由 daemon 猜测 revision 增量。

**自动化:** dozer-core serde、dozer-client、dozerd 路由、MCP tool、app handler 的
端到端 round-trip；断线、超时、旧 tab id、revision 冲突和重复 request id。

---

## T14. 大文件硬性回归与性能门槛

- [x] 稀疏索引按固定缓冲块扫描；任何单行长度下峰值临时内存不随整行长度增长
      (`BufRead::fill_buf` 固定块,不再 `read_until` 整行)。
- [x] Windowed 读取量封顶 `WINDOW_MAX_BYTES + O(buffer)`；超长单行显示前缀和明确
      截断提示(`read_window_capped` 用 `take(max+1)`;`truncated` → 顶部横幅)。
- [x] `SetWindow` 下发链路做集成测试，防止命令因普通 CodeMirror 判据被过滤；
      验收必须看到正文和截断提示，不能只看到行号/滚动条。
      (`windowed_tab_enqueues_set_window_command`)
- [ ] Windowed 全局滚动条与普通 CodeMirror 样式一致，拖动仍映射全局行号。
      (滚动条与全局行号映射已有;像素级样式一致性需真机核对。)
- [x] 索引任务支持取消；tab 关闭、项目切换、文件 revision 变化时旧任务结果失效。
      (`build_cancellable` + `apply_window_index` revision 校验 + 外部变更清索引)
- [x] 流式搜索对超长单行也必须有界；摘要和列号计算不得复制整行。
      (`stream_search` 改分段重叠扫描,增量 UTF-8 字符计数,摘要只取命中附近有限字节)
- [ ] 建立 fixture：300MB 无换行、百万短行、混合 CRLF、跨缓冲区 UTF-8、巨大
      JSONL 单行。(300MB 无换行 + 跨缓冲区 CRLF 已加;百万短行 / JSONL 单行 /
      跨缓冲区 UTF-8 未逐一建 fixture。)

**自动化门槛:**

- [x] `read_window` 最大读取量测试；
- [x] 跨缓冲区换行 offset 测试；
- [ ] 300MB 单行首屏 command 集成测试（允许生成 sparse/temp fixture）；
      (已测 300MB 单行有界窗口读取;完整 command 链路未做端到端。)
- [ ] 搜索取消与结果封顶；(结果封顶已有测试;流式搜索取消未做。)
- [x] revision 变化后旧 index 不得复用。

**真机门槛:** 打开 100MB/300MB/600MB fixture，记录首屏时间和 RSS；RSS 不得接近
文件大小线性增长。数据记录进验收报告，不只写“通过”。

---

## T15. 文档、依赖、打包与最终验收

- [x] 更新旧 JSON 计划：普通 JSON Tree 已由 vanilla-jsoneditor 替代；Streamed
      JSON 是 T8 新实现，不再描述为旧 tree adapter 延续。
- [x] 更新 `CLAUDE.md`/`CODEBUDDY.md` 中编辑器、路由、runtime、资源与大文件约束
      (新增「文件预览路由与查看器」小节)。
- [~] 核对 macOS 打包包含 editor/json-editor/flyfish/html host 资源与字体；全部离线，
      无 CDN、无 404。(editor/json-editor/flyfish 已在包内;html host 是 T7 未做;
      字体为二进制内嵌;web 产物无运行时外链。)
- [x] 核对第三方许可证；运行 `cargo udeps` 或人工确认无残留直接依赖。
      (`cargo machete` 干净;已移除因删除自研 JSON 树而失效的 `sonic-rs` 直接依赖。)
- [ ] 更新人工验收清单到最终目标状态，删除迁移期“预期失败”。
- [~] 执行：
  - `cargo fmt --check`
  - `cargo test -p dozer-app`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets`
  - editor/json-editor 的 typecheck、test、build
  - release macOS app smoke test
  (fmt/test/clippy(dozer-app)/editor 已过;workspace 全量/release smoke 未跑。)
- [ ] 按清单完整真机验收，保存路由矩阵、资源诊断、性能数据和失败截图。
- [ ] 更新总计划完成定义和所有 phase progress；未通过项不得用“已知限制”改名后勾选。

---

## 推荐执行顺序与依赖

```text
T0 文档校准
 ├─ T1 fallback
 ├─ T5 路由 ─ T6 字节安全
 └─ T2 runtime ─ T3 资源闭环 ─ T4 adapter 清理
                         ├─ T7 HTML 隔离 ─ T9 Flyfish 收敛
                         ├─ T8 Streamed JSON
                         ├─ T10 冲突处理
                         ├─ T11 视图恢复
                         └─ T12 Tabular Agent

T13a 协议设计 ─ T13b 导航 ─ T13c 写入
T14 大文件门槛贯穿 T2/T3/T8，最终阻断发布
全部完成后执行 T15
```

可并行边界：

- T1 与 T2 可并行；
- T5/T6 可与 T2 的类型设计并行，但接资源生命周期必须等 T3；
- T13a 可提前设计，T13b/T13c 必须复用 T3 的 suspended 唤醒；
- T14 测试应随相关 task 同步加入，不能集中拖到最后补。

---

## 完成定义

以下全部成立才算文件预览架构重构完成：

1. 所有 fixture 均有内部 viewer、可解释安全降级或明确外部打开页面，不出现空白。
2. backend 描述、runtime、lifecycle 各自只有一份真相；迁移 adapter 字段已删除。
3. CodeMirror、JSON、Flyfish、Tabular、Windowed/Streamed 均受跨项目总预算管理；
   淘汰会真实进入 Suspended，且不会逐帧重建。
4. 启动只物化当前需要的 tab；历史 tab 数量不导致线性启动读取。
5. 脏内容、外部变化、recovery 和 revision 冲突均不会静默丢数据。
6. cursor、selection、scroll、folds 和 Tabular 视图状态能完成持久化/淘汰恢复闭环。
7. Agent 能读取上下文、导航；写入仅在 backend 可写且 revision 匹配时成功。
8. JSONL/NDJSON 使用真实 Streamed backend，坏行局部失败，大文件不构建整文件 DOM。
9. 非 UTF-8 路径字节安全；任何有损展示都只读且不能覆盖原文件。
10. 超大文件和超长单行的索引、读取、搜索、首屏均有界；300MB 单行不会空白，
    不会产生接近文件大小的正文常驻增量。
11. 所有 WebView 资源离线打包、安全域隔离、协议可校验；release 包 smoke test 通过。
12. 自动化、人工路由矩阵、资源诊断和性能记录全部通过并归档。
