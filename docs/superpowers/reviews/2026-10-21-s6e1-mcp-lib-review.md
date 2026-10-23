# S6e-1 审核记录

日期:2026-10-21。被审:`feat/mcp-product`(worktree `../dozer-mcp-product`,顶端 `7b14490e`,4 个功能提交 + 报告,基于 `feat/panel-agent-restore` 的 `adae877b`)。
计划:`plans/2026-10-18-mcp-s6e1-lib.md`。实施报告:该分支内 `reviews/2026-10-21-s6e1-mcp-lib-report.md`。

## 结论

**初审需返工 1 项(F1,阻塞);复审(`faf24115`)已通过,见文末。** 测试、门禁与数字都对得上;但实现者为了让黄金测试在 workspace 构建里也过,在 `install` 的 `write_json` 里**加了全文件键排序**,这改变了 Dozer 实际发布构建的行为,违反本期"Dozer 行为字节不变"的核心约束。

## 我自己实跑的验证(干净快照,独立 `target`)

| 项 | 结果 |
| --- | --- |
| 基线祖先 | `adae877b` 是 HEAD 祖先 ✔;工作区干净 |
| `cargo metadata --locked --offline` | 退出码 0 |
| `cargo test --workspace --no-fail-fast` | 3461 通过 / 3 失败 / 5 ignored;3 失败是 `git_log` 环境失败;3461 + 3 = **3464 = 3436 + 28**,与报告一致 |
| 测试名守恒 | 基线测试名丢失 0,新增 28 |
| `cargo clippy --workspace --all-targets` | 退出码 0;41 条警告(同基线);`dozer-mcp`/`dozer-core` 无警告 |
| `rustfmt --check --edition 2024` 对本期 10 个 `.rs` | 全部干净 |
| `check_migration.py` / `check_panel_boundary.py` | 退出码 0 / ok |
| `check-log-scope.sh` | 只剩 `main` 既有的 `preview_panel.rs:233` |
| `cargo tree -p dozer-mcp -e normal` | 无 `dozer-app` / `dozerd` |
| 黄金夹具 | 只在 `66ccc789` 出现一次,此后未改 ✔(但测试文件本身后来改了,见 F1) |

另手工核对:`ProductId::socket_path()` 与旧 `paths::socket_path()` 等价;`run_main` 的用法文案、缺变量报错文案对默认产品逐字不变;默认服务器名 `rmcp` 保留(与基线 `initialize` 一致);同名保护、返回码 3、`config_path_for_product` 的环境变量前缀、旧入口薄封装都符合计划。

## 发现

### F1 `write_json` 的全文件键排序改变了发布构建的行为(阻塞)
- **事实:** 打包脚本是 `cargo build -p dozer-app -p dozerd -p dozer-hook -p dozer-mcp`,同一次构建里 `mongodb`/`bson`、`tree-sitter` 等依赖打开了 `serde_json` 的 `preserve_order`(我用 `cargo tree -e features -i serde_json` 确认)。所以**实际发布的 `dozer-mcp`,以及 `dozer-app` 进程内调用的 `run_at_with_exe`,在抽库前是保留键序的**:用户 `~/.claude.json` 里别的键保持原顺序,我们写的 `dozer` 条目是 `type, command, args, env` 的插入顺序。
- **现在:** `write_json` 对整个文档递归按字典序重排。我在 workspace 构建里实测:输入 `{"zeta":1,"alpha":{"y":1,"b":2},"mcpServers":{}}`,输出变成 `alpha{b,y}`、`mcpServers`、`zeta`,`dozer` 条目变成 `args, command, env, type`。也就是安装一次 MCP 就把用户整个 `~/.claude.json`(混装了别的 app 状态的大文件)、`opencode.json` 的键序全部改掉。这与计划 Global Constraints "配置文件只增删自己名字的键;别的键原样保留"、"Dozer 行为字节不变"都冲突。
- **根因:** 黄金是在 `cargo test -p dozer-mcp`(无 `preserve_order`,BTreeMap 即字典序)下生成的。我用 `66ccc789` 的原始黄金测试验证:`-p dozer-mcp` 4 条通过,`cargo test --workspace --test default_product_golden` 3 条失败(`install_outputs`、`tools_list`、`initialize`)。实现者没有去问"哪个才是发布构建的基线",而是把产品代码改成排序来迎合黄金,报告偏差 2 把它写成"与默认 BTreeMap 时代一致",这个"一致"只对测试构建成立,对发布构建恰恰是变化。
- **要求的修复:**
  1. 去掉 `write_json` 里的 `sorted_json`,恢复为直接 `serde_json::to_string_pretty(value)`(与基线字节行为一致,键序随构建的 `preserve_order` 而定,和以前完全相同)。
  2. 黄金比较改成与构建无关的方式:`install` 三种产物的黄金也按 `canonical_json`(`Value` 递归规范键序)比较,与 `tools_list`/`initialize` 的做法一致。保留一条断言说明:黄金只钉内容,不钉键序。
  3. 加一条回归测试,钉住"别的键的相对顺序不被我们改动"——`preserve_order` 只在部分构建里开,所以测试用"我们不引入排序"的方式表达:对一个已按**非字典序**排列的输入,在 `cargo test --workspace` 构建里断言输出键序与输入相同(单独 `-p dozer-mcp` 下无法保序,用 `cfg!`/特性探测跳过并注明原因),或直接对 `write_json` 调用点断言"不调用排序"。实现者选一种能真正失败的写法,并说明如何验证它会失败。
  4. 修完后在 `cargo test --workspace` 与 `cargo test -p dozer-mcp` **两种构建**下都全绿,报告的"干净快照"节要写明这两种。
  5. 更正报告偏差 2 的措辞。

## 非阻塞观察

1. **空的本产品会话变量的语义变了。** 旧 `run()` 用 `std::env::var(...)`,`DOZER_SESSION_ID=`(空串)被当作有效 id;新 `run_for` 把空串当"不存在",于是落到安静模式或报错。计划 Review Focus 2 明确要求"存在且非空,与 hook 一致",所以这是计划内的选择,但它是对默认产品的一处行为变化,报告没有披露。建议在报告里补一句,并在测试里钉住(现有 `quiet_mode_is_not_entered_for_an_empty_other_variable` 只覆盖别家的空变量)。
2. **计划点名的 4 条测试没有同名出现:** `a_user_written_entry_with_the_same_name_is_not_overwritten_and_returns_3`(实际为去掉 `_and_returns_3` 的版本)、`uninstall_refuses_to_remove_a_foreign_same_named_entry`、`missing_empty_and_corrupt_files_behave_as_before`、`reinstalling_either_is_idempotent_and_does_not_touch_the_other`。前一条只是改名;后三条我没在新增列表里找到同名或明显等价项。返工时请核对,缺的补上或在报告里说明已由哪条测试覆盖(卸载拒绝删外来同名条目、缺/空/损坏文件、重复安装幂等都是计划的 Review Focus 3、4 点名的行为)。
3. 安静模式构造了 `Client` 值但不连接(偏差 3):`Client::new` 只存路径,我核对属实;`DozerMcpServer::has_tool` 是 `pub` 且仅测试用,可接受。
4. `entry_is_ours` 参数是条目名而非 `ProductId`,与计划签名略有出入,但对两个产品等价(条目名 = 产品名)。

## 未做/不属于本期

- Task 6(Digger 钉版)、合并、tag、推送均未做,符合修订。手动抽检清单在报告里,等整条链路完成后一次性验收。

## 链路状态

Dozer 侧叠分支:S6a Part A → S6b → S6c-1 → S6c-2 → S6d-2 Part A(`adae877b`)→ **S6e-1(`7b14490e`,F1 返工中)**。这是 Dozer 侧最后一环,F1 修完后即可进入你的一次性人工验收。

## 复审(2026-10-21,顶端 `faf24115`,返工提交 `9713afd2` + 报告 `faf24115`)

**结论:通过,F1 与观察 1、2 均已收口。** 干净快照(独立 `target`)实跑:
- `write_json` 已恢复为直接 `to_string_pretty`,`sorted_json` 删除;`src/` 其余无改动,夹具 0 改动。
- `cargo metadata --locked --offline` 0。`cargo test -p dozer-mcp`:全绿(lib 25 + 各集成目标,无 FAILED)。`cargo test --workspace --no-fail-fast`:3466 通过 / 3 失败(`git_log` 环境)/ 5 ignored;3466 + 3 = **3469 = 3464 + 5 条新增**,与报告一致。**两种构建都全绿。**
- **回归测试能真失败:** 我在快照里临时把 `write_json` 改成经 `BTreeMap` 写出,`cargo test --workspace --lib -p dozer-mcp install_does_not_reorder` 立刻 FAILED;还原后在 workspace 构建中该测试实际执行并通过(不是 skip)。单独 `-p dozer-mcp` 构建下它会 skip,这是预期的,报告已写明。
- 补回的测试:`uninstall_refuses_to_remove_a_foreign_same_named_entry`(三种格式,文件字节不变)、`missing_empty_and_corrupt_files_behave_as_before`(我对照基线核实:缺文件新建,空文件/损坏都返回 1,与旧行为一致)、`reinstalling_either_is_idempotent_and_does_not_touch_the_other`、`empty_own_session_variable_is_treated_as_missing`(披露了空串语义变化);`..._and_returns_3` 已改名。
- clippy 退出码 0、41 条(同基线,`dozer-mcp` 无警告);改动文件 `rustfmt --edition 2024` 干净;`check_migration.py` 0;边界脚本 ok。报告的偏差 2 已更正。

**S6e-1 通过。未合并、未打 tag、未推送。Dozer 侧全部审完。**
