# 文件预览 —— 人工检测清单(当前实现)

> 用途:指导人工在真机上检测文件预览重构的**查看 / 编辑 / 搜索 / 大文件 /
> 恢复 / Agent** 行为。本版对应当前实现(CodeMirror + vanilla-jsoneditor 均
> **常开**,老 iced `CodeView` 与自研普通 JSON Tree 已删除),取代早先按
> `--features codemirror` 门控的旧清单。
>
> 关联:`docs/superpowers/plans/2026-09-22-file-preview-architecture-redesign.md`
> (总计划)、`file-preview-phase-{b,c,d}-progress.md`、
> `2026-09-22-file-preview-wrap-up.md`(未完成收尾计划)。
>
> **读表约定(迁移期)**:每行标注 **当前预期 / 目标预期** 两种。检测时按
> **当前预期**判"是否回归";**目标预期**是 wrap-up 计划(T1/T5/T6/T8…)落地后
> 的形态,尚未实现时按"已知缺口"§9 处理,不计入回归。

## 0. 环境与构建

- [ ] `cargo build -p dozer-app` 通过(无需任何 feature;`default = []`,能力
      已常开)。
- [ ] 运行 `cargo run -p dozer-app`,打开任意项目(Files 预览 + Project 右配对
      预览都要测,两个 `PreviewPane` 互相独立)。

**状态与数据位置(macOS,`<config>` = `dozer_core::paths::config_dir()`,通常
在 `~/Library/Application Support/` 下以 `dozer` 结尾的目录;用
`ls ~/Library/Application\ Support | grep -i dozer` 定位):**

| 内容 | 路径 |
|---|---|
| 每个项目的预览 tab 持久化 | `<config>/preview_state/<project_id>.json` |
| 启动进行/完成标记 | `<config>/preview_startup.json` |
| 单文件连续失败计数 | `<config>/preview_failures.json` |
| 脏内容 recovery 快照 | `<config>/preview_recovery/` |

## 1. Fixture 准备

先 `cd` 到某个被测项目根目录,再生成以下文件。fixture 固定放在项目内,确保
Files 文件树可以直接访问;验收完成后可手工删除 `.file-preview-fixtures/`:

```bash
d="$(pwd)/.file-preview-fixtures"
mkdir -p "$d"
printf 'fn main() {}\n'                          > "$d/a.rs"
printf 'print(1)\n'                              > "$d/a.py"
printf '普通中文文本\n第二行\n'                    > "$d/cn.txt"
printf '\xEF\xBB\xBFbom\r\nline2\r\n'            > "$d/bom_crlf.txt"
printf '{"a":1,"b":[2,3]}\n'                     > "$d/ok.json"
printf '{ // c\n "a":1, }\n'                     > "$d/cfg.json5"
printf '{"x":1}\n{"x":2}\n'                      > "$d/rows.jsonl"
printf 'name,age\nann,3\nbob,4\n'                > "$d/t.csv"
printf '# 标题\n**粗体**\n'                       > "$d/doc.md"
printf '<h1>hi</h1>\n'                           > "$d/page.html"
printf 'all:\n\t@echo ok\n'                      > "$d/Makefile"
printf 'FROM scratch\n'                          > "$d/Dockerfile"
printf 'fixture license\n'                       > "$d/LICENSE"
printf 'KEY=value\n'                             > "$d/.env"
printf 'unknown utf-8 text\n'                    > "$d/README_NO_EXT"
printf '\x00\x01\x02\xFF'                       > "$d/unknown.binblob"
printf '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>\n' > "$d/icon.svg"
printf '\xFF\xFEb\x00a\x00d\x00\n\x00'         > "$d/utf16le.rs"     # 合法 UTF-16LE
printf '\xFF\xFEbad utf16\n'                     > "$d/non_utf8.rs"     # 非法/奇数长度 UTF-16LE
printf 'fn main() {\xFF}\n'                       > "$d/invalid_utf8.rs" # 无 BOM 非法 UTF-8
printf 'fn\0main\n'                               > "$d/binary_spoof.rs" # 二进制伪装源码
head -c 2000000 /dev/zero | tr '\0' 'a'          > "$d/long_line_plain.rs" # 2MB 单行:非窗口化降级
head -c 6291456 /dev/zero | tr '\0' 'a'          > "$d/long_line.rs"   # 6MiB 单行:强制 Windowed
head -c 300000000 /dev/zero | tr '\0' 'a'        > "$d/huge.txt"       # 300MB
mkdir -p "$d/archive-src"
printf 'hello\n'                                  > "$d/archive-src/hello.txt"
(cd "$d/archive-src" && zip -q "$d/archive.zip" hello.txt)            # 合法 ZIP
printf 'PK\x03\x04stub'                          > "$d/broken.zip"     # 损坏 ZIP
echo "$d"
```

若环境没有 `zip`,可跳过合法 ZIP 的运行时渲染检查,但仍需保留 `broken.zip` 的
失败降级检查。不要用损坏 ZIP 同时代表“正常压缩包路由”和“加载失败”两种场景。

## 2. 路由矩阵(逐个打开,记录实际后端)

| 文件 | 期望后端(当前) | 期望行为(当前) | 目标后端(wrap-up) | 结果 |
|---|---|---|---|---|
| `a.rs` / `a.py` | CodeMirror | 行号 / 语法高亮 / 可编辑 / ⌘S 保存 | 不变 | |
| `cn.txt` | CodeMirror | 中文正常显示,非方块/空白 | 不变 | |
| `bom_crlf.txt` | CodeMirror | 保存后 BOM 与 CRLF 保留 | 不变 | |
| `ok.json` | JSON Tree(可切 Text) | tab 栏可切「树 / 文本」 | 不变(vanilla-jsoneditor) | |
| `cfg.json5` | CodeMirror(jsonc) | 注释 / 尾逗号着色,可编辑 | 不变 | |
| `rows.jsonl` | Streamed(T8) | 逐行流式视图(窗口化只读);tab 栏可切「原文文本」 | 局部错误节点/展开(T8 剩余) | |
| `t.csv` | Tabular 网格 | 网格显示;tab 栏可切「网格 / 原文」 | 不变 | |
| `doc.md` / `page.html` | Rendered + 可切 Source | 渲染;切 Source 进 CodeMirror | HTML 走 `dozer://html/` 隔离 host(T7) | |
| 图片 / PDF | Rendered(Flyfish) | 正常渲染 | 不变 | |
| `archive.zip` | External → 统一 fallback 页(T1) | 显示类型/路径/原因 + 外部打开;不内嵌 | 不变 | |
| `broken.zip` | External → 统一 fallback 页(T1) | 不崩溃;外部打开可用 | 不变 | |
| `README_NO_EXT` | CodeMirror(内容探测为文本,T5) | 能查看 / 可编辑 | 不变 | |
| `Makefile` / `Dockerfile` / `LICENSE` / `.env` | CodeMirror(文件名规则,T5) | 能查看 / 可编辑 | 不变 | |
| `unknown.binblob` | Unsupported → 统一 fallback 页(T1) | 不崩溃、不显示为可编辑文本;**无**纯文本退路 | 不变 | |
| `icon.svg` | Rendered(Flyfish 图像) | 图像正常;tab 栏可切 CodeMirror XML 源码(T5) | 不变 | |
| `utf16le.rs` | CodeMirror **只读** + 编码提示(T6) | 合法 UTF-16 已解码为正确文字;顶部提示 "UTF-16 只读",⌘S 被拒 | 不变 | |
| `non_utf8.rs` / `invalid_utf8.rs` | CodeMirror **只读** + 有损提示(T6) | 有损文字只读;顶部提示只读原因,⌘S 被拒,磁盘 hash 不变 | 字节安全只读 | |
| `binary_spoof.rs` | Unsupported → 统一 fallback 页(T6) | 内容二进制凌驾源码扩展名;不显示为可编辑文本,不崩溃 | 不变 | |
| `long_line_plain.rs` | CodeMirror **只读纯文本** | 关闭换行 / 高亮 / 折叠,不要求全局滚动条 | 不变 | |
| `long_line.rs` | Windowed **只读** | 全局行号基数、可滚到任意行 | 不变 | |
| `huge.txt` | Windowed **只读** | 打开不卡死、不全量读内存 | 不变 | |

## 3. 编辑器能力(CodeMirror tab)

- [ ] 行号与真实文件行一致;滚动条拖到底,最后一行真实可见(不是空白)。
- [ ] 代码折叠(语言支持时)可展开 / 收起。
- [ ] ⌘F 搜索:host 内搜索面板弹出、命中高亮、next/prev。
- [ ] ⌘R 替换(可写 tab):current / all 替换生效并标脏。
- [ ] 中文 IME 组合、候选、提交正常。
- [ ] 输入 / 删除 / Enter / Tab / 剪贴板(⌘C/⌘X/⌘V)/ undo(⌘Z)/ redo(⌘⇧Z)。
- [ ] ⌘S 保存成功、dirty 星号清除;保存后磁盘内容与 BOM/换行约定一致。
- [ ] 外部修改文件:干净 tab 自动重载;脏 tab 出现冲突条(「保留我的修改」/
      「重载磁盘」,重载需再点一次确认)。保留后若磁盘又变,保存被拒并重新提示
      (wrap-up T10)。
- [ ] 主题深 / 浅切换,编辑器跟色;JetBrains Mono + 中文回退正确。
- [ ] 编辑器聚焦时 ⌘`=` / ⌘`-` / ⌘`1`(或 Ctrl 版)缩放 UI。

## 4. 大文件 / Windowed

- [ ] 打开 `huge.txt` 前后记录活动监视器 RSS;确认没有接近 300MB 的正文全量
      常驻增量。若要判断增长曲线,另用 100MB / 600MB fixture 对照,不要只凭
      单个样本断言“非线性”。
- [ ] 窗口化:右侧全局滚动条可拖到任意行,行号是全局基数。
- [ ] 滚到已持有窗口边界附近,自动加载相邻窗口(约 400ms 节流)且视觉不跳。
- [ ] 窗口化 tab 只读:输入无效、⌘S 无效。
- [ ] 单行 `long_line_plain.rs`:关闭换行 / 高亮 / 折叠但仍完整可查看;
      `long_line.rs`:进入 Windowed,仍可查看且全局行号正确。

## 5. 大文件整文件搜索(Windowed)

- [ ] 在窗口化 tab 按 ⌘F → 内容区顶部出现搜索条(查询框 + `n/m` + 上/下 + ×)。
- [ ] 输入 query 回车 → 命中数为**整文件**(不只当前窗口)统计。
- [ ] 点上/下一条:自动装对应窗口并 reveal 到命中行。
- [ ] 点 × 关闭搜索条。
- [ ] 记录:搜索条不支持 Esc 关闭 / 自动聚焦(host webview 持焦点,见 wrap-up T-备注)。

## 6. Agent / MCP 上下文

- [ ] CodeMirror tab 选中一段 → MCP `get_preview_context` 返回
      path / cursor / selection / revision / mode / read_only / visible 行。
- [ ] 表格 tab:返回 `tabular`(sheet / scroll_row / scroll_col)。
- [ ] 记录:`preview_editor_reveal/select/replace` 是否有外部(daemon/MCP)调用
      入口 —— 目前**无调用方**(见 wrap-up T3),预期“Agent 写入”不可用。

## 7. 恢复 / 资源 / 安全启动

- [ ] 在同一项目打开多个文件(含大文件)→ 重启 → **只加载当前项目当前文件**,
      其余 tab 是 Suspended 壳;点开其它 tab 才加载。
- [ ] 多项目各开若干文件 → 累计驻留受预算约束(超预算时后台干净 tab 被休眠)。
- [ ] 脏 tab(未保存)→ 编辑后等约 1.5s(触发快照)→ `kill -9 <dozer pid>` →
      重启 → 正文恢复且重新标脏。
- [ ] 脏 tab 快照后**外部改动该文件** → 重启 → 出现冲突提示,不静默覆盖。
- [ ] 安全启动:先
      `printf '{"status":"in_progress","started_ms":1}' > "<config>/preview_startup.json"`
      再启动 → 进入安全启动(仅恢复 tab 壳,不自动加载文件)。
- [ ] 手动构造损坏 recovery:向 `<config>/preview_recovery/` 丢一个非法 JSON
      文件 → 启动不 panic、忽略该快照。
- [ ] 对同一不可读文件反复打开(≥3 次)→ 出现外部打开按钮 / 降级提示。注意:
      当前失败态已经有通用「在系统应用中打开」按钮;这里还要记录 reason、重试
      和纯文本退路是否缺失,用于 T1。

## 8. 无网络

- [ ] 断网后打开 / 编辑 / 搜索 / 切换主题,预览与编辑器仍正常(无 CDN 依赖)。

## 9. 已知缺口(预期会失败,勿误报为回归)

以下项**尚未实现**,检测时按“记录 / 不阻塞”处理,详见
`2026-09-22-file-preview-wrap-up.md`:

- 普通 Failed 态已有通用「在系统应用中打开」按钮;T1 起压缩包 / 未知二进制改走
  统一 fallback 页(类型/路径/原因 + 重试/纯文本只读/外部打开),不再 host Flyfish。
- 未知 UTF-8 文本、`Makefile` / `Dockerfile` / `LICENSE` 已进 Code(T5);
- 脏 tab 外部变更:已提供冲突条与「保留我的 / 重载磁盘」(二次确认),T10 落地;
  仅“异常退出后 recovery 与磁盘冲突时进同一 UI”尚未接线。
- Agent 写操作(reveal/select/replace)无 daemon/MCP 调用入口(T13);休眠 tab
  的 Agent 唤醒未接线。
- JSONL/NDJSON 已路由为 `PreviewKind::Streamed`(T8):复用窗口化有界行视图,可切
  「原文文本」回退;结构化"局部错误节点/每行展开"尚未做。
- 超大 `.json` 仍按 JSON Tree 处理,未按预算自动降级到 Text/Windowed/Streamed(T8)。
- 表格无 reveal cell/range,Agent 无“选中单元格”(T12)。
- HTML 已走 `dozer://html/` 隔离 host(无脚本 sandbox iframe,T7)。Flyfish 已建立
  `proj/panel/tab/doc` 绑定并把 title/搜索状态/失败事件迁到通用 envelope(T9);
  但旧零散 JS 搜索注入尚未删除(parity 未做),资源管理器接线仍 partial(T3/T9)。
- 窗口化搜索条无 Esc 关闭 / 程序化聚焦(末尾备注)。
- scroll anchor 仍是逻辑锚点,未精确还原像素(T11)。
- 非法 UTF-8 / UTF-16 展示已有只读提示且保存恒拒绝(T6);**仍**可能经过
  `fetch().text()` 解码,所以显示的是替换字符(有损),但不会写回原文件。
  完整 byte-safe 十六进制查看器未做(计划允许"T1 外部打开"或只读展示二选一)。

## 10. 结论记录

- 通过项:______________________________
- 阻塞项(附复现步骤 / 截图):______________________________
- 结论:□ 达到收尾前置条件  □ 需先修复阻塞项
