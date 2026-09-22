# 文件预览 Phase C 进度(大文件 / 恢复 / 资源管理)

> 对应计划:`docs/superpowers/plans/2026-09-22-large-file-session-resource-management.md`。
> 2026-09-22。

## 已完成(纯逻辑模块,不碰热点文件)

### Task 1:文件策略决策器 `preview/file_policy.rs`
- `TextFilePolicy { EditableCode, ReadOnlyHighlighted, ReadOnlyPlain, Windowed }`。
- 系数 9x/6x/4x;绝对护栏 30/64/128MiB;单行规则 >100KiB 关 wrap、>1MiB 关
  高亮/折叠、>5MiB 强制 Windowed。
- `decide_text_policy(profile, budgets) -> TextPolicyDecision`,带
  `estimated_bytes` / `guard_bytes` / `capabilities` 与**可展示的具体 reason**
  (含文件大小、最长行、预算),不是"大文件"bool。纯函数、`saturating_mul` 防溢出、
  单次强制不在此改全局预算(调用方语义)。
- 测试:低/高预算、30/64/128MiB 边界、单行三档、溢出饱和、单调性。

### Task 2:流式搜索与稀疏行索引 `preview/large_text.rs`
- `stream_search(path, query, SearchOptions) -> SearchOutcome`:逐行流式,返回
  1-based 行列 + 有限摘要;命中列表封顶但**保留匹配总数**与 `truncated`。
- `LineIndex::build/build_cancellable`(每 N 行一个 byte offset,可取消)、
  `offset_for_line`(最近索引 seek 后有限扫描)、`read_window`(目标附近有界
  窗口 + 全局行号)、`is_valid_for(revision)` 失效判定。
- 测试:行/列(含 UTF-8 字符列)、CRLF、命中封顶但总数保留、空 query、逐行
  offset 精确、无尾换行、空文件、窗口钳位、可取消。

### Task 4:全局资源管理器 `preview/resources.rs`
- `ResourceManager` 跨项目登记 `ViewerRegistration`(估算字节、重型 WebView、
  active/dirty/has_recovery/saving/agent_writing、LRU `last_accessed`)。
- `try_reserve(bytes, heavy, current_project) -> Granted | NeedEviction(keys) |
  Denied`:内存与重型名额**同时**满足;不偷偷淘汰,只报告淘汰谁(销毁后再归还
  预算)。
- `eviction_order`:后台项目干净 → 后台有 recovery 脏 → 当前项目非活动干净 →
  当前项目有 recovery 脏,组内 LRU。不可淘汰:active/saving/agent 写入/
  无 recovery 脏 tab。
- `diagnostics()` 诊断快照。测试:预算/名额、不可淘汰跳过、脏有无 recovery、
  淘汰顺序、LRU、超大单件拒绝、release 幂等、重复登记不叠加。

## 仍未完成(需接热点文件/运行期,建议在无并行改动时做)

1. **Task 3 Windowed Viewer**:把 `file_policy`/`large_text` 接进 `PreviewTab`/
   runtime —— Rust 读窗口、CodeMirror 只持窗口、全局行号基数、边界加载、
   Agent reveal 未加载行。需要改 `view.rs`/`runtime.rs`/editor 协议(增加
   windowed 模式与 `set_window`)。
2. **Task 5 Suspended 恢复**:扩展 `preview_state` schema、启动只恢复壳、并发 1、
   懒加载。需改 `workspace/state.rs`/`view.rs`。
3. **Task 6 Dirty recovery snapshot** `preview/recovery.rs`:版本化 manifest、
   防抖原子写、启动恢复/冲突/清理。可作为独立纯模块先做,再接 `PreviewTab`。
4. **Task 7 安全启动与运行时反馈**:启动进行/完成标记、失败计数、ready latency。
5. 把 `file_policy` 的只读/窗口化结论回写到 `PreviewBackend`/路由(`router.rs`),
   让大文件不再无脑进 CodeMirror 整载。

## 验证

- `cargo check -p dozer-app --all-targets`(默认与 `--features codemirror`):通过。
- `cargo test -p dozer-app`:默认 **1231 passed / 0 failed**、feature **1213
  passed / 0 failed**(并行工作已修掉先前的 icon 既有失败;另有 1 ignored)。
  新增:`file_policy` 9、`large_text` 10、`resources` 11。
- `cargo fmt --check`、`cargo clippy` 干净(仅既有 `file_history.rs` warning)。
