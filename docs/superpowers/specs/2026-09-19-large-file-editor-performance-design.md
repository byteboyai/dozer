# 大文本文件（GB 级）打开性能优化设计

## 背景与动机

代码编辑器（`crates/dozer-app/src/code_editor/`，包装 iced 官方 `iced_widget::text_editor::TextEditor`）打开文件时，`CodeView::new` 调 `text_editor::Content::with_text(text)`，底层 `cosmic_text::Buffer::set_text` 在**尚未设置视口高度**（`height_opt == None`）时就无条件调用 `shape_until_scroll`，导致 shaping 窗口退化成 `[0, ∞)`——**把整份文档一次性 shape 完**，同步阻塞 UI 线程。这是 iced/cosmic-text 内部行为，不是 Dozer 自己写的逻辑。

Dozer 目前的应对是 `native_editor.rs` 里一道 `MAX_NATIVE_EDITOR_BYTES = 256 * 1024`（256KB）的硬阈值：超过就直接回退到 flyfish（wry webview 渲染的只读预览），并留了注释记录实测代价（约 14ms/KB，267KB≈3.5s，7.5MB≈98s）。这个阈值：

- 挡不住真正的问题——flyfish 路径本身对超大文件**没有任何 size guard**，行为未经验证，很可能是把同一个"整读进内存"的问题转嫁到一个更黑盒的组件（JS 渲染、独立进程）。
- 即使把根因 shaping bug 修掉，编辑器自己的 undo 栈（`EDIT_HISTORY_LIMIT = 60`，整文件 `String` 全量快照）在大文件上依然是独立的内存问题，和 shaping 是否修好无关。

Dozer 的核心原则是「预览优先于编辑」：GB 级文本文件（多为日志/数据导出）用户的真实诉求是"打开能看、能滚动、能搜索"，而非"像小文件一样直接编辑保存"。因此本设计不追求让编辑器在任意大小文件上都保留完整编辑能力，而是按文件大小分档，只读档换取"不管多大都能快速打开"。

## 目标 / 非目标

**目标：**

1. 修复 shaping 根因，使打开耗时只正比于"当前可见行数"，不再正比于"文件总行数"——所有档位（含现状 <256KB 的小文件）都受益。
2. 新增只读大文件档：整读入内存渲染（按机器可用内存动态定上限），超过上限再降级为分块加载，复用 Tabular Viewer 已有的 `truncated` + 横幅 + "加载更多"先例。
3. 只读档下全文搜索复用 Files 面板已有的 `grep-searcher`（流式），不在内存里对整个 `String` 做线性扫描。
4. 文件读取异步化（不阻塞 UI 线程），打开期间显示 loading 态。

**非目标（本期裁掉）：**

- 只读档下的编辑能力（含部分区域可编辑等折中方案）——不做。
- 真正的无限大文件支持（mmap + 流式行索引 + 自建虚拟化渲染组件）——过度工程，不符合"预览优先"的核心原则，超出 `FULL_LOAD_MAX` 上限（见下）后用分块加载兜底即可，不追求单文件无上限。
- flyfish（webview 预览）路径自身的大文件优化——本设计让原生编辑器覆盖绝大多数大文件场景，flyfish 只在原生编辑器构造失败（如非文本/权限错误）时才会被触达，其自身的 GB 级行为不在本期处理范围。
- 分块加载档的"增量向前/向后翻页"或"跳转到任意字节偏移"——一期只支持"从头加载，点击加载更多，向后追加"，不支持回退/跳转。

## 分档策略与阈值

三档，判定发生在 `native_editor.rs`（`fs::metadata` 取文件大小，同步、足够快）：

```rust
// crates/dozer-app/src/preview/native_editor.rs

/// 低于此值：现状全功能编辑（含 undo/保存）。固定值，不随机器内存缩放——
/// 这一档的瓶颈是 undo 栈本身的设计（60 份整文件 String 快照），不是单次
/// 读取的内存代价；调大这个值只会让"编辑几次撑爆内存"的场景更容易触发。
const EDIT_MODE_MAX_BYTES: u64 = 20 * 1024 * 1024; // 20MB

/// [EDIT_MODE_MAX_BYTES, FULL_LOAD_MAX) 是"只读·整读"档:全文读入内存,
/// 只读,虚拟化渲染。FULL_LOAD_MAX 见 full_load_max_bytes()。
///
/// >= FULL_LOAD_MAX 是"只读·分块"档:首屏只读入 FULL_LOAD_MAX 字节,
/// UI 提供"加载更多"继续追加。

/// 按机器可用内存动态计算整读档上限。
///
/// 公式: total_ram × 10% ÷ 3, 钳到 [256MB, 4GB]。
/// - ×10%: 单文件不占用超过系统总内存的十分之一,给其他应用/Dozer自身状态留余量
/// - ÷3: 读取(read_to_string 缓冲)+ 校验/lossy 转换的临时拷贝 + 灌入
///   cosmic-text Vec<BufferLine> 后的常驻拷贝,峰值约为原始文件的 2~3 倍,
///   按 3 倍留安全边际
/// - 下限 256MB: 8GB 内存的机器公式算出来会贴地板,给个保底
/// - 上限 4GB: 即使 128GB 工作站,单文件也不建议整读 4GB+ 进内存,
///   超过一律走分块档
fn full_load_max_bytes() -> u64 {
    let total = system_total_memory_bytes(); // sysinfo,见"依赖"
    ((total as f64 * 0.10 / 3.0) as u64).clamp(256 * 1024 * 1024, 4 * 1024 * 1024 * 1024)
}
```

常见机器换算：8GB→273MB、16GB→546MB、32GB→1.07GB、64GB→2.13GB、128GB→4.27GB→钳到 4GB(触顶)。下限 256MB 只在总内存低于约 7.5GB 的机器上才会实际触发。

`full_load_max_bytes()` 每次打开文件时现算一次（查系统总内存是毫秒级操作，不缓存；不用"当前可用内存"是因为它会随其他应用占用波动，导致同一份文件先后两次打开可能分到不同档位，体验不一致——用总内存是稳定量）。

## 根因修复（shaping bug）

**这是实现阶段唯一需要先花一个 spike 验证、此设计不预先拍板具体技术路径的部分**，因为问题出在 iced/cosmic-text 内部，Dozer 无法直接重排自己代码里的调用顺序来解决。候选路径（二选一，由实现时验证可行性决定）：

1. **先挂载后换内容**：`CodeView` 构造时先用极小 placeholder 文本（如空串）建 `Content`，让 widget 走过一次真实 `layout()`（此时 `iced_widget::text_editor` 内部理应已经拿到视口 bounds），再把全文内容通过某种增量 API 换进去，使得真正装载大文本时 `height_opt` 已非 `None`。
2. **构造前声明尺寸**：排查 `iced_widget::text_editor::Content` / 底层 cosmic-text `Editor` 是否有"构造时/构造后立即声明视口尺寸"的公开口子，在灌入文本前调用。

两条路径都要满足同一个验收标准：**打开一个只读大文件档的文件，首次可交互耗时只应随"当前可见行数"变化，不应随"文件总行数"线性增长**（用相对基准衡量，见"测试策略"）。若两条路径都不可行，需要回到设计阶段重新评估（如短期 vendor 一份 patched `text_editor` widget），但预计不会走到这一步——沿用 mod.rs 里"2026-09 已移除自建 iced-code-editor"的既定决策，不主动重新引入 vendor 分支。

这个修复对编辑档（<20MB）文件同样受益：现状 256KB→3.5s 的卡顿门槛会直接消失，小到中等文件几乎瞬开。

## 只读模式

`CodeView` 新增 `read_only: bool` 字段（构造时按分档结果传入）：

- **Action 过滤**：`read_only == true` 时，分发层拦截 `Insert`/`Delete`/`Paste` 等编辑类 `Action`（保留 `Move`/`Select`/`Scroll`），使得底层 widget 状态永远不会被写入。
- **跳过 undo 栈**：`Snapshot` 记录逻辑（mod.rs 里 `EDIT_HISTORY_LIMIT` 相关代码）整段短路，只读 tab 不分配、不追加快照——避免"只读大文件也占着 60 份潜在快照的内存预算"这种无意义开销（虽然不会写入触发快照，但短路更清晰，不依赖"反正不会被调用"的隐式假设）。
- **UI**：保存入口（⌘S、保存按钮）对只读 tab 隐藏/禁用；顶栏加"只读 · 文件过大 (X MB)"提示 chip，让用户明确知道为什么不能编辑。

## 打开数据流（异步化）

```
用户触发打开
  → fs::metadata(path) 取大小（同步，足够快）
  → 按大小分档（EDIT_MODE_MAX_BYTES / full_load_max_bytes()）
  → tab 立即挂载，显示 loading 态（骨架/spinner）
  → tokio::spawn 异步读取：
      - 编辑档 / 只读整读档：tokio::fs::read(path)，整文件读完
      - 只读分块档：tokio::fs::read 前 full_load_max_bytes() 字节，
        按合法 UTF-8 字符边界截断（不完整的多字节字符尾部丢弃，
        下次"加载更多"从该字节偏移续读）
  → 读取完成 Message 回传主循环 → 灌入 CodeView（触达根因修复点）
  → loading 态解除
```

读盘搬到异步任务后，UI 线程只在"读取完成、灌入 buffer 触发 shaping"这一步可能有短暂耗时——这一步的耗时正是根因修复要解决的部分（收敛到只和可见行数相关）。

## 分块加载档 UI

复刻 Tabular Viewer 已有的 `truncated: bool` + 提示横幅 + "加载更多"交互（`tabular/mod.rs` 先例），不发明新 UI 模式：

- `PreviewTab` 的只读大文件状态记录 `loaded_bytes: u64`、`total_bytes: u64`、`truncated: bool`。
- 顶部（编辑器上方）显示"仅加载前 X MB，共 Y MB，[加载更多]"横幅，`truncated` 为真时才渲染。
- 点击"加载更多"：异步续读下一个 `full_load_max_bytes()` 大小的分块（从 `loaded_bytes` 对应的字节偏移开始，同样做 UTF-8 边界对齐），追加到已有 buffer。若 iced `text_editor::Content` 没有暴露"追加文本而不重建整个 Buffer"的增量 API，退化为"重新读取 `[0, loaded_bytes+chunk)` 并重建 Content"——由于根因修复后 shaping 代价只和可见窗口相关，重建整个 Content 本身不再是性能瓶颈，只是重复读盘的 I/O 代价，可接受。

## 搜索

- **编辑档（<20MB）**：维持现状，用 `text_editor` widget 内置 find（`PreviewFind*` 消息链路），量级小无需优化。
- **只读大文件档（整读 + 分块）**：复用 Files 面板已有的 `grep-searcher`/`grep-regex` 依赖，对磁盘上的原文件做流式搜索（不依赖已加载进内存的部分，即使是分块档，搜索范围也是整份文件，而不是"已加载的那部分"）。命中结果（字节偏移/行号）转换成行号后：
  - 若命中行已在已加载范围内：直接跳转高亮。
  - 若命中行超出已加载范围（分块档搜到后面的内容）：提示"命中内容超出已加载范围，点击加载更多"，不自动追加全部剩余内容（避免搜索操作意外触发整文件读入内存，违反分块档存在的意义）。

## 错误处理

| 场景 | 处理 |
|---|---|
| 非 UTF-8 内容 | 沿用现有 `String::from_utf8_lossy` fallback，只读档同样适用；分块档额外要求分块边界按合法 UTF-8 字符边界对齐（见"打开数据流"）。 |
| 磁盘读取失败 / 权限错误 | 沿用现状：`Err` → 回退 flyfish 只读预览。 |
| 读取期间文件被外部修改/删除 | 按现状 `Err` 处理，不做并发保护（YAGNI，遇到真实报告再迭代）。 |
| 内存不足 | 不做主动探测。`full_load_max_bytes()` 已经按可用内存留了安全边际（总内存 10% ÷ 3），正常路径不会逼近系统极限；超出设计预算的极端情况（如系统本身内存已被其他进程占满）交给 Rust 分配失败的默认行为，不特殊处理。 |
| `sysinfo` 查询总内存失败（极端环境） | 退化为固定默认值（如 512MB，介于经验值下限与常见 16GB 机器算出的档位之间），不 panic、不阻塞打开流程。 |

## 测试策略

- **分档函数单测**：给定文件大小 + mock 总内存（`full_load_max_bytes` 接受可注入的 total-memory 参数或用 trait/闭包桩替换 `sysinfo` 调用），断言三档边界判定正确，含 256MB/4GB 钳位触发的两种情况。
- **UTF-8 边界对齐单测**：构造一个多字节字符（如中文）恰好落在分块边界上的 fixture，断言截断不产生非法 UTF-8、下一分块从正确字节偏移续读且不丢字符、不重复。
- **shaping 性能回归测试**：清理 `code_editor/highlighter.rs` 里硬编码作者本机路径（`/Users/chrischiang/Projects/WorkProjects/...`）的 `#[cfg(test)] mod repro` 测试——改成运行时生成大文件 fixture（临时目录，程序生成指定行数的文本），断言"打开耗时不随行数线性增长"：用相对基准而非绝对时间（如 1 万行 vs 100 万行耗时比值应远小于 100 倍），避免和 CI 机器性能绑死。
- **只读 Action 过滤单测**：构造 `read_only: true` 的 `CodeView`，分发 `Insert`/`Delete` Action，断言底层文本内容未变、`Snapshot` 栈未增长。
- **分块加载"加载更多"单测**：mock 一个已知内容的大文件，验证分块读取、追加、`loaded_bytes`/`truncated` 状态更新正确。
- 真实 GB 级文件的端到端打开耗时不做自动化断言（受机器性能影响大），列入人工验收清单：在开发机上验证打开 1GB/3GB/6GB 级别 txt/log 文件的实际耗时与可交互性。

## 依赖

```toml
# crates/dozer-app/Cargo.toml
sysinfo = "0.32"  # 查询系统总内存，跨平台（与"mac 先发但架构留门"的既有裁决一致）
```

`grep-searcher`/`grep-regex`/`ignore`（Files 面板全文搜索已用）、`tokio`（workspace 已有，本设计是其在编辑器打开路径上的首次实际使用）均为复用现有依赖，不新增。
