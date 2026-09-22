# 文件预览重构 —— 人工验收清单(Phase B/C/D)

> 目标:在真机上确认 `codemirror` feature 的查看/编辑/搜索/大文件/恢复/Agent
> 行为达到规格要求,作为"把 feature 转默认开启、再执行 Phase D 删除旧实现"的
> 前置条件。
>
> 关联:`2026-09-22-file-preview-architecture-redesign.md`(规格 §"可观测性与
> 验收")、Phase B/C/D 进度文档。

## 0. 运行方式

```bash
# dev(热改方便)
cargo run -p dozer-app --features codemirror

# 或 release(更接近发布行为)
cargo build --release -p dozer-app --features codemirror
./target/release/dozer
```

无 feature 的默认构建应保持旧行为;本清单只验收 feature 开启态。

## 1. Fixture 准备

在某个被测项目里生成以下文件(可用脚本):

```bash
d=$(mktemp -d)
printf 'fn main() {}\n'                         > "$d/a.rs"
printf 'print(1)\n'                             > "$d/a.py"
printf '普通中文文本\n第二行\n'                    > "$d/cn.txt"
printf '\xEF\xBB\xBFbom\r\nline2\r\n'           > "$d/bom_crlf.txt"
printf '{"a":1,"b":[2,3]}\n'                    > "$d/ok.json"
printf '{ // c\n "a":1, }\n'                    > "$d/cfg.json5"
printf '{"x":1}\n{"x":2}\n'                     > "$d/rows.jsonl"
printf 'name,age\nann,3\nbob,4\n'               > "$d/t.csv"
printf '# 标题\n**粗体**\n'                       > "$d/doc.md"
printf '<h1>hi</h1>\n'                          > "$d/page.html"
printf '\xFF\xFEbad utf16\n'                    > "$d/non_utf8.rs"
head -c 2000000 /dev/zero | tr '\0' 'a'         > "$d/long_line.rs"   # 2MB 单行
mkfile 300m "$d/huge.txt" 2>/dev/null || head -c 300000000 /dev/zero | tr '\0' 'a' > "$d/huge.txt"
echo "$d"
```

## 2. 路由矩阵(逐个打开,记录实际后端)

| 文件 | 期望后端 | 期望行为 | 结果 |
|---|---|---|---|
| `a.rs` / `a.py` | CodeMirror | 行号/高亮/可编辑/⌘S 保存 | |
| `cn.txt` | CodeMirror | 中文正常、非方块 | |
| `bom_crlf.txt` | CodeMirror | 保存后保留 BOM 与 CRLF | |
| `ok.json` | JSON Tree(可切 Text) | 树/文本切换 | |
| `cfg.json5` | CodeMirror(jsonc) | 注释/尾逗号着色 | |
| `rows.jsonl` | Streamed | 每行一 root,可查看 | |
| `t.csv` | Tabular | 网格;可切原文 | |
| `doc.md` / `page.html` | Rendered + 可切 Source | 渲染;切 Source 进 CodeMirror | |
| 图片/PDF | Rendered(Flyfish) | 正常渲染 | |
| `.zip` | 外部打开 fallback | 不空白 | |
| 未知文本 | Flyfish(Phase A 契约) | 能查看 | |
| `non_utf8.rs` | CodeMirror 只读 | 不可编辑、可查看 | |
| `long_line.rs` | Windowed 只读 | 全局行号/滚动到任意行 | |
| `huge.txt` | Windowed 只读 | 不全量卡死 | |

## 3. 编辑器能力(CodeMirror tab)

- [ ] 行号与真实文件行一致;拖到底最后一行可见。
- [ ] 代码折叠(语言支持时)可展开/收起。
- [ ] ⌘F 搜索准确,命中高亮;⌘R 替换(可写 tab)。
- [ ] 中文 IME 输入正常(组合、候选、提交)。
- [ ] 输入/删除/Enter/Tab/剪贴板/undo/redo。
- [ ] ⌘S 保存;保存后 dirty 标记清除;外部改文件 → 干净 tab 自动重载、脏 tab 冲突提示。
- [ ] 主题深/浅切换,编辑器跟色;JetBrains Mono + 中文回退。

## 4. 大文件 / Windowed(Phase C)

- [ ] `huge.txt` 打开不把整文件读进内存(活动监视器观察)。
- [ ] 窗口化:全局行号基数、右侧全局滚动条可拖到任意行、滚到边界自动加载相邻窗口。
- [ ] ⌘F 走**整文件**流式搜索,命中可跳转(装窗 + reveal)。
- [ ] 只读:不可编辑、⌘S 无效。
- [ ] 单行 2MB 文件:关换行/关高亮但仍可查看。

## 5. Agent / daemon(Phase B Task 6)

- [ ] 选中一段 → MCP `get_preview_context` 返回 path/cursor/selection/revision/mode/read_only/visible 行。
- [ ] Agent reveal 到折叠内行 → 自动展开并选中。
- [ ] `preview_editor_replace` 带过期 revision → 被拒(revision 冲突)。

## 6. 恢复 / 资源(Phase C)

- [ ] 开多个大文件 tab → 重启 → **只加载当前项目当前文件**,其余是 Suspended 壳;切换时才加载。
- [ ] 多项目各开若干大文件 → 累计驻留受预算约束;超预算休眠后台干净 tab。
- [ ] 脏 tab(未保存)→ 关闭应用 → 重启 → 恢复正文并标脏;若磁盘已变 → 冲突提示(不静默覆盖)。
- [ ] 连续加载失败的文件 → 出现外部打开按钮;达到阈值提示降级。
- [ ] 启动标记:上次强退后重启 → 安全启动(仅恢复壳,不自动加载)。

## 7. 无网络

- [ ] 断网后编辑器/预览仍正常(无 CDN 依赖)。

## 8. 结论记录

- 通过项:______________________________
- 阻塞项(附复现步骤/截图):______________________________
- 结论:□ 可把 `codemirror` 转默认开启并进入 Phase D 删除  □ 需先修复阻塞项

> 通过后再执行:把 `crates/dozer-app/Cargo.toml` 的 `default = ["codemirror"]`,
> 然后按 Phase D Task 5/7 删除老 iced `CodeView` 与自研普通 JSON Tree(每步保留
> 功能对照测试与回退提交点)。
