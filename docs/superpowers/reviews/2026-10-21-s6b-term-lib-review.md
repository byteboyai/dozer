# S6b 终端控件库 `dozer-panel-term` 开发结果审核

日期:2026-10-21。

审核对象:[2026-10-18-agent-s6b-term-lib.md](../plans/2026-10-18-agent-s6b-term-lib.md) 的 Dozer 侧实现,分支 `feat/panel-term`(worktree `../dozer-panel-term`),HEAD `5716e721`,4 个提交,基于 S6a 顶端 `72a3544c`(满足总设计 §0 第 9 条"叠分支")。实现者的报告在该分支里:`docs/superpowers/reviews/2026-10-21-s6b-term-lib-report.md`。

## 结论

**S6b 通过。** 实现本身质量很高,"行为不变"有强证据。审核中发现的两件事已于 2026-10-21 经用户同意由审核人直接修复:**B1**(`check_migration.py` 失败,报告称通过与事实不符)修复于 `726f86c7`;分支上 4 个 `product_isolation` 失败的根因是 S6a 遗漏了被 `.gitignore` 吞掉的文件(S6a 审核文档 N2),已在 `feat/product-id` 修复为 `b7763bb8`,并把本分支变基上去。修复后 `feat/panel-term` 顶端为 `726f86c7`,**S6c-1 从这个顶端切出**。下面"验证记录"是修复前的第一次实测,修复后的实测见文末"修复与复验"。

**验证记录(审核人在 `feat/panel-term` worktree 实跑,输出与退出码分别落盘):**

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace --no-fail-fast` | 退出码 101;**3375 通过 / 4 失败 / 5 ignored**。4 个失败全是 `dozerd` 的 `product_isolation`(`app_host_is_unavailable_when_the_feature_is_off`、`events_do_not_cross_between_two_daemons`、`isolated_roots_sockets_and_dbs_do_not_overlap`、`second_daemon_on_the_same_root_exits_without_touching_data`),原因是找不到 `product_daemon` 示例二进制(S6a N2) |
| `cargo clippy --workspace --all-targets` | 退出码 0;39 条警告(与 S6a 顶端相同),无一在本分支改动的文件里 |
| `cargo fmt --check` | 退出码 1;9 个文件,与本分支改动的文件**无交集**,属既有问题 |
| `scripts/audit/check_panel_boundary.py` | 通过 |
| `scripts/i18n/check_migration.py` | **失败**(S6a 顶端:退出码 0、`unresolved=0`;S6b 顶端:退出码 1、`unresolved=1`)→ B1 |
| 干净快照 `git archive HEAD` + `cargo metadata --locked --offline` | 退出码 0(新库的 `Cargo.lock` 条目已随提交带上) |
| 新库依赖闭包 | 无 `dozer-app/client/core`、`dozerd`、`wry`、`portable`、`tauri`;源码里无 `crate::app/theme/assets/workspace`、无硬编码字体名 |

## 已独立验证、通过的部分

**1. 黄金测试在旧代码上成立,且不是恒真。** 报告说黄金文件"在搬任何代码之前生成",但提交历史里黄金测试与模型搬迁在同一个提交(`1d123afa`),从历史上无法证明先后。审核人做了更直接的验证:把 S6b 分支里的黄金测试原样植入 **S6a 的旧 `term_model.rs`**,夹具文件取自 S6b 分支,在旧代码上跑 → **通过**(`1 passed`)。再把夹具第 20 行改一个字符 → **失败**,并精确指出 `line 20` 的差异。也就是说:夹具反映的是抽库前的真实行为,新代码又通过同一份夹具,所以模型的渲染相关数值没有变。验证用的临时改动已在 S6a worktree 还原,确认 `git status` 干净、HEAD 仍是 `72a3544c`。

**2. 47 个测试逐个按名字都在新库里。** 旧的 `term_model`(31)+ `term_view`(16)共 47 个测试名,与新库的测试名集合做差集,**旧测试里新库没有的为空**。新库另有 13 个新增测试,外加 1 个被 `#[ignore]` 的零尺寸测试,合计 60 通过 + 1 ignored,与报告一致。`dozer-app` 为 1286 通过 / 2 ignored,等于 1333 − 47。

**3. 报告的测试总数算术经我的数字复核成立。** 报告写 3365,我跑出 3375,差 10:报告没有加 `--no-fail-fast`,`cargo test` 在失败的 `product_isolation` 那个测试二进制处停下,后面的二进制没跑。但增量一致:S6a 顶端 3366 通过 → S6b 顶端 3375 + 4(失败,实际应通过)= 3379,**+13 通过、+1 ignored**,与报告相同。

**4. 接口与计划逐字一致。** `TermStyle { font, font_size, line_height_factor }` 及 `scaled_font_size / cell_width / line_height_px / grid_size`;`TermEvent { Input, Scroll, SelStart, SelUpdate }`;`view<'a, T, M>(model, focused, target, preedit, style, map: fn(T, TermEvent) -> M)`,约束 `T: Copy`、`M: Clone`。计划要求的 6 个视图测试与 6 个模型钉死测试都在(其中 `grid_size_of_zero_or_negative_pane_is_zero_by_zero` 以 `grid_size_of_zero_negative_or_nan_pane_does_not_panic` 的名字存在,覆盖了 NaN)。

**5. Dozer 侧适配层干净。** `term/term_view.rs` 对外签名(`view` / `grid_size`)不变,四种事件一一映射回 `Message`,`app.rs` / `app/view.rs` / `terminal.rs` 的调用点**零改动**;`term/term_model.rs` 只剩一行 `pub use`。样式缓存在 `OnceLock` 里没有问题:字号与行高是编译期内嵌常量,会变的 UI 缩放由 `scaled_font_size()` 每次换算时实时读取,Ctrl +/- 缩放不会失效。

**6. 文本绘制配置没有被搬丢。** 新旧两处的 `canvas::Text` 构造都是 `..canvas::Text::default()`,没有显式 `Shaping`,与改造前完全相同,不构成回归。

## B1 · P1:`check_migration.py` 失败,报告称"仍为 `unresolved=0`"与事实不符(已关闭:`726f86c7`)

> **处置与更正(2026-10-21):** 已在 `EXCLUDED_FILES` 加入 `crates/dozer-panel-term/src/model.rs`(旧路径那条保留不动),并把报告里那句不实的话改成事实。**下面"修改要求"第 2 条(重新生成 `inventory.json`)是我写错了,没有执行,也不该执行:** `build_inventory.py` 是覆盖式重写,实测重新生成会产生约 2.3 万行无关 diff(+9090/−14326),并把 `migrated` 清零,抹掉后续迁移任务写回的记录;而 `check_migration.py` 是在内存里重跑 `build_inventory.build()` 判断的,**根本不读**已提交的 `inventory.json`,所以只改排除表就够。修复后实测 `check_migration.py` 退出码 0、`inventory: total=2677 unresolved=0`(分支上和干净快照上都是)。下面保留原始发现供追溯。

**位置:** `scripts/i18n/build_inventory.py` 的 `EXCLUDED_FILES`(第 110 行列着旧路径 `crates/dozer-app/src/term/term_model.rs`);`docs/dozer-v2/i18n/inventory.json`;被判为"未分类 UI 候选"的是 `crates/dozer-panel-term/src/model.rs:295` 的 `#[must_use = "PTY 应答字节需要显式处理:实时输出写回 daemon,快照回放丢弃"]`。

**问题:** 旧路径 `term_model.rs` 是被明确排除的"非面向用户的源码"(测试辅助、内部常量)。模型搬到 `crates/dozer-panel-term/src/model.rs` 之后,新路径没有进入排除表,里面这条给开发者看的属性提示被扫描器当成未分类 UI 候选,`unresolved=1`,门禁失败。报告的"门禁与登记"一节写了 `check_migration.py` 仍为 `unresolved=0`,实测不成立(S6a 顶端是 0,S6b 顶端是 1)。

**修改要求:**
1. 在 `scripts/i18n/build_inventory.py` 的 `EXCLUDED_FILES` 里加入 `crates/dozer-panel-term/src/model.rs`;旧路径那条随文件变成一行转调,可留可删,二选一,在提交说明里写明。
2. 重新生成 `docs/dozer-v2/i18n/inventory.json`(用 `build_inventory.py`,不要手改),把重新生成的结果提交。注意先看重新生成的 diff 是否只涉及终端模型这一处,若夹带了无关条目的变化,先停下说明原因。
3. 报告里关于 `check_migration.py` 的那句话改成实测事实。

**验收:** `python3 scripts/i18n/check_migration.py` 退出码为 0 且输出 `unresolved=0`;`python3 scripts/audit/check_panel_boundary.py` 仍通过;重新生成的 `inventory.json` 的 diff 只含与 `dozer-panel-term/src/model.rs` 相关的条目。

## 次要观察(不阻塞,不要求修)

- **库的黄金测试依赖另一个 crate 里的夹具文件:** `model.rs` 的 `TERM_GOLDEN` 用 `CARGO_MANIFEST_DIR/../dozer-app/tests/fixtures/term_golden.txt`。这是计划自己指定的位置,在本仓库里没有问题;但库的测试不再自包含,将来若把库整体迁出(如总设计 §0 第 2 条提到的迁入 bytehost)需要一并带上夹具。
- **`grid_size` 对 `INFINITY` 输入与 `font_size == 0` 会得到极大网格(`as usize` 饱和):** 公式按计划要求与改造前逐字一致,NaN 与负值已钳到 0,所以这是既有行为,不是回归。本期不改。
- **被 `#[ignore]` 的既有缺陷已如实记录:** `new_and_resize_to_zero_or_one_cell_do_not_panic`——`alacritty_terminal 0.26` 不支持 0 行/0 列网格。按计划没有顺手修,处理正确。

## 剩余门禁

1. 修 B1 并提交(在 `feat/panel-term` 上)。
2. S6a 修复 N2 之后,把 `feat/panel-term` 叠到修复后的 `feat/product-id` 顶端(S6c-1 尚未开始,此时变基代价最低)。
3. 人工验收仍按总设计 §0 第 9 条推迟到整条 Dozer 侧链路完成之后,清单已保留在报告里。
4. 以上完成后,S6c-1 从 `feat/panel-term` 顶端切出。

---

## 修复与复验(2026-10-21,经用户同意由审核人直接修复)

**修复:**

| 项 | 做法 | 提交 |
| --- | --- | --- |
| S6a N2(`product_daemon.rs` 被 `.gitignore` 吞掉) | 提交该文件,`.gitignore` 加 `!crates/dozerd/examples/` 例外 | `b7763bb8`(`feat/product-id`) |
| 叠分支 | `git rebase --onto b7763bb8 72a3544c feat/panel-term`,无冲突;变基前后的差异恰好是 N2 的两个文件 | S6b 四个提交的哈希随之改变 |
| B1 | 把 `crates/dozer-panel-term/src/model.rs` 加进 `build_inventory.py` 的 `EXCLUDED_FILES`,并更正报告里的不实陈述;`inventory.json` 未重新生成(原因见 B1 处置) | `726f86c7`(`feat/panel-term`) |

修复后 `feat/panel-term` 顶端为 **`726f86c7`**,基于 `feat/product-id` 顶端 `b7763bb8`。

**复验(审核人实跑):**

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace --no-fail-fast`(S6b worktree) | 退出码 0;**3379 通过 / 0 失败 / 5 ignored**(修复前 3375 通过 + 4 失败 = 3379,总数一致,4 个失败转为通过) |
| `product_isolation` 的 4 个测试 | 全部 `ok` |
| 黄金测试 `model::tests::golden_corpus_visible_lines_cursor_and_modes` | `ok` |
| `dozer-panel-term` / `dozer-app` | 60 通过 / 1 ignored;1286 通过 / 2 ignored(与修复前一致) |
| `cargo clippy --workspace --all-targets` | 退出码 0;39 条警告,与之前相同 |
| `check_migration.py`(分支与干净快照) | 退出码 0,`unresolved=0` |
| `check_panel_boundary.py` | 通过 |
| 干净快照 `cargo metadata --locked --offline` | 退出码 0 |
| S6a 顶端干净快照 `cargo test -p dozerd --no-fail-fast` | 退出码 0;508 通过 / 0 失败 / 1 ignored;4 个隔离测试都实际运行 |

**结论:S6b 通过,代码侧已无阻塞项。** S6c-1 从 `feat/panel-term` 顶端 `726f86c7` 切出(总设计 §0 第 9 条)。人工验收仍按该条推迟到整条 Dozer 侧链路完成之后,清单保留在实现者报告里。

**留下的非阻塞项(见上文"次要观察"):** 库的黄金测试依赖 `dozer-app/tests/fixtures/` 里的夹具;`grid_size` 对 `INFINITY` 与 `font_size == 0` 的既有行为;被忽略的零尺寸既有缺陷测试。
