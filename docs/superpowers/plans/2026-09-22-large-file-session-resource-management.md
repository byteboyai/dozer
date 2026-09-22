# 文件预览重构 Phase C：大文件、恢复与资源管理 Implementation Plan

**Goal:** 让单个大文件和跨项目累计打开文件都受动态预算控制；启动只恢复 tab 壳，
大文件降级后仍可查看、搜索和被 Agent 跳转，脏内容可安全休眠与恢复。

**Depends on:** Phase A；CodeMirror 整载/IPC 使用 Phase B 接口。

## Task 1：文件策略决策器

**Files:**

- Create: `crates/dozer-app/src/preview/file_policy.rs`

```rust
pub enum TextFilePolicy {
    EditableCode,
    ReadOnlyHighlighted,
    ReadOnlyPlain,
    Windowed,
}
```

- [ ] 实现 9x/6x/4x 估算系数和 30/64/128MiB 初始绝对护栏。
- [ ] 实现 100KiB 关 wrap、1MiB 关高亮/折叠、5MiB 强制 Windowed 的单行规则。
- [ ] 低配/高配、阈值边界、溢出算术、不同类型测试全部用纯输入。
- [ ] RouteReason 展示具体文件画像、预算和降级项；不得只显示“大文件”。
- [ ] 用户单次强制尝试不能改变全局预算，也不能跳过硬安全上限。

## Task 2：Rust 流式搜索与稀疏行索引

**Files:**

- Create: `crates/dozer-app/src/preview/large_text.rs`
- Reuse/refactor: existing large-file grep search

- [ ] 流式搜索不加载全文，返回行、列、有限摘要；命中列表封顶并标记 truncated。
- [ ] 建每 N 行一个 byte offset 的稀疏索引；构建可取消、可渐进、不能阻塞首屏。
- [ ] UTF-8 边界、CRLF、超长行、文件截断/替换期间读取都有测试。
- [ ] jump-to-line 从最近索引 seek，再有限扫描到目标；错误不返回错误行。
- [ ] 索引按 file revision 失效，不能用于已经替换的文件。

## Task 3：Windowed Viewer

- [ ] Rust 读取目标前后有界窗口；CodeMirror 只持有窗口内容。
- [ ] 自定义全局行号基数，cursor/selection 转换回全局坐标。
- [ ] 滚到窗口边界触发相邻窗口加载并保持视觉 anchor。
- [ ] Agent reveal 未加载行：索引定位、加载窗口、ready 后 select。
- [ ] Windowed 恒只读；复制、搜索、跳转可用，折叠/全文 undo 明确禁用。
- [ ] 超大/非法 UTF-8 文件退到 byte-safe streamed viewer 或外部打开。

## Task 4：全局 PreviewResourceManager

**Files:**

- Create: `crates/dozer-app/src/preview/resources.rs`
- Modify: App/project/workspace lifecycle

- [ ] 登记所有项目 resident viewer 的 estimated bytes、heavy webview、active、dirty、
  pinned reason、last_accessed。
- [ ] 新加载前 reserve；不足时 LRU 淘汰后台项目 clean，再淘汰当前项目非活动 clean。
- [ ] current/saving/agent-write/no-recovery-dirty 不可淘汰。
- [ ] 销毁实际 viewer 后才归还预算；Loading 取消和失败也必须归还 reservation。
- [ ] max_heavy_webviews 与内存预算同时满足；数量不是内存预算替代品。
- [ ] 提供诊断 snapshot，测试跨项目顺序、pin、取消和重复 release。

## Task 5：真正 Suspended 的 tab 恢复

**Files:**

- Modify: `preview_state.rs`, PreviewPane restore/open/select

- [ ] 启动只读取 descriptors，全部建立 Suspended tab 壳。
- [ ] UI 首屏建立后仅 queue 当前项目当前 tab，启动 load concurrency 固定 1。
- [ ] 后台项目不建 WebView、不读全文；用户切项目/tab 时再 reserve/load。
- [ ] clean tab suspend 前序列化逻辑 anchor，恢复时检查 file revision 后加载。
- [ ] 文件不存在/权限变化/路由变化进入 Failed，不删除历史 tab。
- [ ] 多项目各十个大文件测试只出现一个首次 load，启动工作量不随 tab 数线性增加。

## Task 6：Dirty recovery snapshot

**Files:**

- Create: `crates/dozer-app/src/preview/recovery.rs`

- [ ] 定义 versioned manifest：path、base revision、editor revision、encoding、换行、
  view state 和 snapshot 文件。
- [ ] 编辑后 debounce 写同目录安全缓存/Dozer recovery 目录，临时文件原子替换。
- [ ] snapshot 成功后 resource manager 才允许休眠 dirty tab。
- [ ] 正常保存且 revision 对齐后删除 recovery；新编辑不能被旧保存回调清除。
- [ ] 启动发现 recovery：磁盘未变则恢复；磁盘也变则冲突，不静默选择。
- [ ] 恢复失败/磁盘满保留 resident 和错误提示，不假装已经保护。
- [ ] 正常退出、崩溃模拟、损坏 manifest、旧版本、并发保存测试。

## Task 7：安全启动与运行时反馈

- [ ] 写启动进行/完成标记；连续未完成时只恢复壳，不自动加载问题文件。
- [ ] 单文件连续失败计数，提供纯文本只读、Windowed、外部打开。
- [ ] 记录 ready latency、加载峰值估算和 route reason；不得上传或记录文件内容。
- [ ] 首版只用启动能力快照做预算；运行时 memory-pressure 作为后续增强接口，不能
  在未验证时引入频繁模式抖动。

## Phase C 验收

- [ ] 大文件任何档位都能查看、搜索、跳转或明确外部打开。
- [ ] 多项目累计驻留不突破预算；切换时无脏数据丢失。
- [ ] 启动历史 tab 再多也只加载一个活动文件。
- [ ] Agent 能跳转 Windowed 未加载行。
- [ ] recovery、冲突、安全启动压力测试通过。
- [ ] 回填 master plan Phase 5–6。

