# 文件预览 Loading 状态与非阻塞动画 Implementation Plan

**Goal:** 在文件预览所有可能产生可感知等待的阶段统一显示
`byteui::feedback::math_curve` loading 效果，并保证文件探测、索引、解析、搜索、
WebView 初始化等耗时工作不占用 UI 线程，使动画在整个等待期间持续流畅。

**关联设计:**

- `docs/superpowers/specs/2026-09-22-file-preview-architecture-redesign.md`
- `docs/superpowers/plans/2026-09-22-file-preview-wrap-up.md`
- `docs/superpowers/analysis/file-preview-acceptance-checklist.md`

本计划只负责 loading 生命周期、非阻塞执行和视觉反馈，不借机重写 viewer。本计划
产生的状态应兼容 wrap-up T2 的统一 `PreviewRuntime`；如果 T2 尚未实施，可先用最小
过渡字段落地，但字段名和转换必须按本文定义，随后原样迁入 runtime。

---

## 1. 当前问题

1. Tabular 的首次解析和 sheet 懒加载已经使用
   `math_curve::loading_hint(Curve::RoseThree, ...)`。
2. 旧 `active_tab.loading` 路径也有“正在打开文件…”动画，但 CodeMirror/Windowed
   正常路径通常不会进入该字段。
3. Windowed 大文件会先创建空 CodeMirror，再后台建立 `LineIndex`；索引完成和首个
   `SetWindow` 下发前，tab 已被标成 `Ready`，因此用户看到空编辑器而不是 loading。
4. CodeMirror、vanilla-jsoneditor、Flyfish 的 WebView 是原生子视图。只在 iced
   内容树里画动画但仍让 WebView 可见，动画会被 WebView 覆盖。
5. `profile_file`、路由判定或任何同步文件操作如果发生在 UI 线程，即使画出了
   loading widget，`AnimatedLoader` 也收不到 `RedrawRequested`，动画仍会冻结。
6. 后台结果目前主要依靠 project/panel/tab id 路由；同一路径快速刷新、关闭重开或
   mode 切换时，还需要 generation/revision 防止旧结果结束新 loading。

---

## 2. 不变量

- UI 线程只做状态转换、轻量队列操作和渲染；不得读大文件、建索引、解析工作簿、
  全文搜索或等待 worker。
- 先把 tab 切到 Loading 并请求重绘，再启动后台任务。
- loading 期间目标 WebView 必须尚未创建或保持不可见，确保 iced 动画没有被原生
  子视图遮挡。
- loading 结束条件是“首个可用画面已准备好”，不是“任务已经 spawn”或“WebView
  已创建”。
- 所有异步结果携带 `(project_id, panel, tab_id, generation)`；不匹配的结果丢弃。
- tab 关闭、项目切换、刷新、mode 切换会取消旧任务；取消是正常结束，不显示错误。
- Failed、External、Unsupported 使用 fallback 页面，不无限停留在 loading。
- `math_curve::loading_hint` 使用自驱动 `AnimatedLoader`；不额外增加全局 60fps
  subscription。组件离开树后应自动停止请求重绘。

---

## 3. 状态模型

新增明确的加载阶段，而不是继续扩张单一 `bool loading`：

```rust
pub enum PreviewLoadStage {
    Idle,
    Profiling,
    Reserving,
    CreatingHost,
    Reading,
    Indexing,
    LoadingWindow,
    Parsing,
    Searching,
    SwitchingMode,
}

pub struct PreviewLoadState {
    pub stage: PreviewLoadStage,
    pub generation: u64,
    pub started_at: Instant,
    pub progress: Option<PreviewLoadProgress>,
}

pub struct PreviewLoadProgress {
    pub completed: u64,
    pub total: Option<u64>,
}
```

约定：

- `BackendState::Loading` 表示 viewer 尚未提供首个可用画面；具体文案由
  `PreviewLoadStage` 决定。
- 后台搜索若覆盖内容区，可使用 `Searching`；已有内容仍可操作时，使用局部搜索条
  loading，不把整个 backend 从 Ready 退回 Loading。
- `progress` 只在底层能低成本提供时显示，不为百分比额外扫描文件。
- generation 每次 open/retry/reload/mode switch `+1`；后台任务捕获启动时值。

阶段与文案映射保持集中：

| Stage | 文案 |
|---|---|
| Profiling | 正在识别文件… |
| Reserving | 正在准备预览资源… |
| CreatingHost | 正在启动预览… |
| Reading | 正在读取文件… |
| Indexing | 正在索引大文件… |
| LoadingWindow | 正在加载文件内容… |
| Parsing | 正在解析文件… |
| Searching | 正在搜索整个文件… |
| SwitchingMode | 正在切换预览模式… |

---

## T1. 统一 loading 视图组件

- [ ] 在预览模块增加 `preview_loading_view(load_state)`，内部统一调用：

  ```rust
  byteui::feedback::math_curve::loading_hint(
      byteui::feedback::math_curve::Curve::RoseThree,
      label,
      48.0,
  )
  ```

- [ ] `workspace/view.rs` 的 Tabular 和旧 `active_tab.loading` 分支改为调用该组件，
      避免文案、尺寸和曲线选择散落。
- [ ] 若有 progress，在动画下方增加有限频率更新的文本；不要每读一个 buffer 都向
      UI 发消息，最多 100ms 更新一次。
- [ ] loading 占满内容区，但保留 tab bar，使用户可以切换或关闭 tab。
- [ ] loading 不接管全局 modal，不阻塞其他项目、终端和面板交互。
- [ ] 增加 reduce-motion 兼容：若未来主题/设置提供减少动态效果，显示静态曲线帧和
      文案；本任务不要求新增设置入口。

**自动化:** 每个 stage 映射到非空文案；未知/Idle 不产生 loading view。

---

## T2. 建立可靠的 loading 转换 API

- [ ] 在 `PreviewPane` 提供集中入口：
  - `begin_load(tab_id, stage) -> generation`
  - `advance_load(tab_id, generation, stage)`
  - `finish_load(tab_id, generation)`
  - `fail_load(tab_id, generation, PreviewError)`
  - `cancel_load(tab_id)`
- [ ] 只有 generation 匹配的 advance/finish/fail 才能修改 tab。
- [ ] `begin_load` 同步更新状态后立即让调用方返回事件循环；不得在该函数内部执行
      I/O 或等待。
- [ ] `finish_load` 只能在首个可用画面准备好后调用。
- [ ] `fail_load` 进入统一 Failed/fallback 页面并清理 pending command、index/search
      session 和资源预留。
- [ ] 逐步淘汰直接写 `tab.loading = true/false` 和零散
      `begin_shell_load(); finish_shell_load();` 连续调用。

**自动化:** generation 过期、取消后回调、失败后重试、重复 finish 均保持合法状态。

---

## T3. 确保动画具备运行机会

这是本计划最重要的非功能任务。

- [ ] 审计打开文件调用栈，将以下工作全部移出 UI 线程：
  - `profile_file` 和超长首行探测；
  - 文件读取/编码检测；
  - `LineIndex::build_cancellable`；
  - `read_window`；
  - JSON/Tabular 解析；
  - 全文件搜索；
  - recovery 快照的大正文读写。
- [ ] 阻塞文件/CPU 工作统一使用 `tokio::task::spawn_blocking`；异步任务不得直接在
      tokio worker 上运行长时间同步循环。
- [ ] `Message::OpenPath` 当帧只创建 shell、设置 `Profiling` 并返回；实际 profile
      结果通过 app message 回灌后再决定 route/backend。
- [ ] 不持有 `App`、`Workspace`、`PreviewPane`、ResourceManager mutex 或 WebView
      pool 锁跨越 I/O/解析。传入 worker 的数据必须是拥有所有权的 path、配置快照和
      cancellation token。
- [ ] worker 进度回报限频，EventLoopProxy 发送失败视为 app/tab 已退出并终止任务。
- [ ] 禁止在 UI handler 中 `block_on`、同步 join、等待 channel 或轮询任务完成。
- [ ] 增加慢任务测试钩子/fixture，证明 loading 已至少绘制两帧后任务才完成。

**性能验收:** 人工注入 2 秒 profile/index/parse 延迟时，动画持续更新，窗口可移动，
tab 可切换，终端仍可输入。

---

## T4. Windowed 大文件打开流程

目标状态序列：

```text
Profiling → Reserving → CreatingHost → Indexing → LoadingWindow → Ready
```

- [ ] Windowed route 确定后保持 `BackendState::Loading`，不要在创建 editor host 时
      立即 `finish_shell_load`。
- [ ] loading 期间不要把 Windowed WebView 加入 visible desired list；可预创建为
      hidden，但首选先显示 iced loading，待 host ready 再开始索引。
- [ ] host `ready` 后转为 `Indexing`，后台构建带 cancellation 的稀疏索引。
- [ ] index 完成后转为 `LoadingWindow`，在后台读取首个有界窗口。
- [ ] 首个 `SetWindow` 必须成功进入待下发队列；WebView 已存在并完成 dispatch 后，
      再将其设为 visible 并 `finish_load`。
- [ ] 如果平台层无法确认 `evaluate_script` 执行完成，则增加 editor
      `window_applied { generation, start_line }` ACK；以 ACK 作为 Ready 边界。
- [ ] 正文可见前不能出现“空 CodeMirror + 行号 1 + 全局滚动条”的中间态。
- [ ] 相邻窗口加载不覆盖整个 viewer：保留当前窗口，在边缘显示轻量局部 loading；
      新窗口失败时继续保留旧内容并显示错误。
- [ ] 超长单行继续遵守 `WINDOW_MAX_BYTES`，索引、窗口读取、搜索均不得整行分配。

**自动化:** 300MB 无换行 fixture 的状态顺序；首个 SetWindow/ACK 前仍是 loading；
关闭 tab 后索引结果被丢弃；旧 generation 不覆盖重新打开的 tab。

---

## T5. 普通 CodeMirror 与 Source 模式

- [ ] 普通文本打开使用 `Profiling → CreatingHost → Reading → Ready`。
- [ ] WebView 未回 `ready` 前隐藏，iced 显示 loading。
- [ ] 对页面自行 `fetch` 文件的现状，增加 `document_loaded` 事件；`ready` 只代表 host
      JS 初始化完成，不能代表正文已经可见。
- [ ] `document_loaded` 携带 revision、字节数和可选错误；Rust 收到后才 finish。
- [ ] Markdown/HTML/SVG 从 Rendered 切 Source 使用 `SwitchingMode`，目标 host 的
      首帧确认前保持原 viewer 或显示 loading，不先展示空编辑器。
- [ ] 保存、刷新和外部变更重载若仍有旧内容可显示，优先保留旧内容并使用局部状态，
      不闪成全屏 loading。

**自动化:** 空文件、小文件、读取失败、host ready 但 fetch 尚未完成、快速切换 mode。

---

## T6. vanilla-jsoneditor

- [ ] JSON Tree 打开使用 `Profiling → CreatingHost → Reading/Parsing → Ready`。
- [ ] JSON host 增加区分 `ready`、`document_loaded`、`failed` 的事件；解析完成前保持
      hidden，避免空白树或半构造 DOM 覆盖 loading。
- [ ] Tree ↔ Text 切换使用 `SwitchingMode`，并通过 generation 丢弃旧 host 结果。
- [ ] 大 JSON 在进入 vanilla-jsoneditor 前先执行预算策略；超预算改走
      Text/Windowed/未来 Streamed，而不是让主线程或 WebView 长时间解析后才失败。

**自动化:** JSON 解析慢、非法 JSON、快速 Tree/Text 往返、超预算降级。

---

## T7. Tabular 首次解析与 sheet 懒加载

- [ ] 保留现有 math_curve 视觉，但统一使用 T1 组件和 T2 状态 API。
- [ ] 首次打开为 `Parsing`；完成 `TabularLoaded` 并得到首个可显示 sheet 后 Ready。
- [ ] sheet 懒加载时保留 tab bar：只在网格内容区显示“正在加载工作表…”，不遮住
      sheet 切换和关闭入口。
- [ ] 工作簿解析、sheet 加载继续使用 `spawn_blocking`；取消后不回填旧 sheet。
- [ ] 空工作簿直接显示稳定空态，不停留 loading。

**自动化:** 大 CSV/XLSX 动画不中断；空工作簿、解析失败、sheet 快速切换、关闭后
结果抵达。

---

## T8. Flyfish、HTML 与其他 Rendered viewer

- [ ] 为 Flyfish/隔离 HTML host 增加统一 `ready/document_loaded/failed` 信号；仅收到
      `document_loaded` 后显示 WebView。
- [ ] 图片、PDF、Office、Markdown、HTML 各自使用 `CreatingHost` 和 `Reading`；
      viewer 内部还有解析阶段时使用 `Parsing`。
- [ ] loading 期间 WebView `set_visible(false)` 或不加入 desired pool；不能依赖 iced
      绘制顺序盖住原生子视图。
- [ ] 加载失败销毁/隐藏 WebView 后进入 fallback 页面，保证错误 UI 不被盖住。
- [ ] 外链、相对资源和网络超时不能让 loading 永久存在；本地文件默认离线，设置
      明确超时与失败事件。

**自动化:** host 永不 ready 的超时、资源 404、坏 PDF/Office、Markdown 渲染失败。

---

## T9. 大文件全文搜索

- [ ] 搜索开始后只在搜索条/结果区域显示 math_curve 小尺寸 loading，正文窗口保持
      可见和可滚动。
- [ ] `stream_search` 在 `spawn_blocking` 中执行并支持 cancellation/generation；新查询
      自动取消旧查询。
- [ ] 搜索循环按固定缓冲块扫描，不能用会为超长单行分配整行的 `BufRead::split`。
- [ ] 命中结果和 progress 限频回灌；结果数量封顶但继续统计时仍可取消。
- [ ] 空查询立即取消并清除 loading；失败显示局部错误，不把整个 preview 置 Failed。

**自动化:** 快速连续输入、300MB 单行、取消、tab 关闭、搜索失败、命中封顶。

---

## T10. 资源等待、恢复与重试

- [ ] ResourceManager reserve 未立即满足时使用 `Reserving`；若需淘汰其他 viewer，
      当前 tab 保持可取消 loading。
- [ ] reserve denied 进入可解释 fallback/失败页，不能无限动画。
- [ ] Suspended tab 被选择时进入对应 stage；recovery 读取和冲突分类在后台执行。
- [ ] Failed 点击重试生成新 generation，旧任务结果不得结束新 loading。
- [ ] 安全启动只恢复 shell，不自动启动 loading 动画或后台加载；用户主动打开后才开始。

**自动化:** 预算等待、denied、淘汰后重试、recovery 慢读、连续失败和安全启动。

---

## T11. 取消、超时与可观测性

- [ ] 每个长任务使用 cancellation token；至少在每个读取 buffer、索引 chunk、解析批次
      检查一次。
- [ ] tab close、reload、mode switch、项目关闭和 app shutdown 触发取消。
- [ ] host 创建、host ready、document loaded 分别设置合理超时；超时进入 retryable
      Failed，不永久转圈。
- [ ] 日志记录 stage、generation、等待毫秒、读取字节、取消/失败原因，不记录正文。
- [ ] 记录 `loading_started → first_frame → ready` 三段延迟，区分“动画没机会画”和
      “后台任务本身慢”。
- [ ] debug 诊断页显示当前 loading task、stage、elapsed 和 cancellation 状态。

---

## T12. 最终测试与真机验收

### 自动化

- [ ] 状态机表驱动测试覆盖所有 stage、成功、失败、取消、重试和过期 generation。
- [ ] WebView desired/visible 测试：Loading 不可见，document/window ACK 后可见。
- [ ] 慢 worker 测试：任务执行期间 UI 至少产生多次 redraw，完成后动画退出。
- [ ] UI handler 审计测试/约束：不得直接调用大文件读取、索引和解析入口。
- [ ] Windowed、JSON、Tabular、Flyfish 的首个可用画面边界测试。
- [ ] `cargo test -p dozer-app`、workspace test、clippy、fmt 和前端 test/build 全部通过。

### 真机

- [ ] 普通源码：短暂显示“正在打开文件…”，随后正文出现，无空白 WebView 闪烁。
- [ ] 300MB `huge.txt`：显示“正在索引大文件…”，动画持续流畅，随后显示正文前缀和
      截断提示。
- [ ] 大 JSON：解析期间动画流畅；超预算按策略降级。
- [ ] 大 CSV/XLSX：解析和 sheet 切换均有动画，窗口/终端仍响应。
- [ ] 图片/PDF/Markdown/HTML：WebView 内容完成前显示动画，不被空白子视图覆盖。
- [ ] 全文件搜索：正文保留，搜索区域动画流畅，可取消。
- [ ] loading 期间反复切 tab、关闭 tab、切项目；无崩溃、无旧结果串台、无永久转圈。
- [ ] 人工给 profile/index/parse 注入 2 秒延迟，确认动画不是静态截图，窗口可拖动，
      终端输入不卡顿。

---

## 推荐实施顺序

```text
T1 统一视图
  → T2 状态 API
  → T3 UI 线程清障
      ├─ T4 Windowed
      ├─ T5 CodeMirror
      ├─ T6 JSON
      ├─ T7 Tabular
      ├─ T8 Rendered/Flyfish
      ├─ T9 搜索
      └─ T10 资源/恢复
  → T11 取消、超时、诊断
  → T12 全量验收
```

T4 应优先落地，因为它已有明确的“空编辑器等待索引”问题，也是验证动画不被 WebView
覆盖、后台索引不阻塞 UI、首个窗口 ACK 边界三项机制的最小完整样板。其余 viewer
复用同一状态 API 和 visible gating。

---

## 完成定义

1. 文件预览所有超过一个可感知帧的等待均有一致的 math_curve loading 或局部 loading。
2. 动画期间主窗口、tab 切换、关闭、终端输入保持响应；人工 2 秒延迟下动画持续运动。
3. UI 线程不执行文件探测、大文件读取、索引、解析或全文搜索。
4. 原生 WebView 不覆盖 loading；首个可用文档 ACK 前保持隐藏。
5. 成功、失败、取消、超时和重试都有确定终态，不出现永久 loading。
6. 异步结果按 generation 校验，快速切换/关闭/重开不会串台。
7. Windowed 大文件不会再出现“只有行号/滚动条的空白编辑器”中间态。
8. 300MB 单行、大 JSON、大 CSV/XLSX、Rendered viewer 和全文搜索均通过自动化与
   真机非阻塞验收。
