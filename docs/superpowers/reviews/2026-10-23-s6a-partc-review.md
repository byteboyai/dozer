# S6a Part C(Task 7–11)开发结果审核

> **更正与修复(同日):** ① B1 的"路径分叉"只在设了 `DIGGER_DATA_DIR` 时成立;macOS 默认下 `directories` 没有 state_dir,`state_dir()` 回落到与 `config_dir()` 同一目录,两处巧合相同。迁移/WAL 未接线仍成立。② 修复时发现审核遗漏的 **B5:GUI 从未 `install` 产品身份,真连 `diggerd` 会被握手判为别的产品**。③ B1–B5 与"撤回 eprintln 一条"已在 `feat/diggerd` 提交 `2286533` 修复,最终代码 423 通过/0 失败,clippy 仅基线警告,详见 Digger 仓库报告末尾"审核修复"。待审核人复审。

日期:2026-10-23。对象:Digger 仓库 `../digger-s6a`,分支 `feat/diggerd`,HEAD `12b8bdd`(基于 `fd17623`,6 个提交,22 个文件 +2300/−159)。计划:[2026-10-18-agent-s6a-daemon-identity.md](../plans/2026-10-18-agent-s6a-daemon-identity.md) Part C。

## 结论

**不通过。** 有一项阻塞缺陷(B1)使 Task 8 的核心目标在生产里没有生效,实现者的报告把它写成了"完成"。门禁数字属实,但门禁测不到这个缺陷。

## 我实跑的验证(`git archive HEAD` 解到独立目录,patch 路径改绝对路径)

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace --no-fail-fast` | 退出码 0;**411 通过 / 0 失败 / 0 ignored**(与报告一致) |
| `cargo clippy --workspace --all-targets` | 退出码 0;仅 `block v0.1.6` 一条基线警告 |
| 改动文件 `rustfmt --check --edition 2024` | 干净 |
| 实跑 `diggerd`(隔离 `HOME` 与 `DIGGER_DATA_DIR`) | 起得来,监听 `data/state/diggerd.sock`,建库 `data/state/digger.db`,SIGTERM 正常退出 |

## B1(P1,阻塞):生产路径从未调用迁移与 WAL,且 GUI 与守护进程打开的是两个不同的文件

- `crates/desktop/src/projects/mod.rs:127` 仍是 `ProjectHost::open(&dir.join(paths::PROJECTS_DB_FILE))`,**没有改成 `ProjectHost::open_in`**。`open_in`(迁移 + WAL)在整个仓库里只有测试调用。后果:旧 `projects.db` 里的项目与记忆在升级后不会迁入,WAL 也从未设置;计划 Review Focus 第 3 点(迁移幂等、并发)形同虚设。计划 Task 8 的 Files 里明确列了要改 `projects/mod.rs`,diff 里没有它。
- 路径分叉:GUI 打开 `<config_dir>/digger.db`(`ProjectsState::open_in(paths::config_dir())` 再 `join`),守护进程用 `product().db_path()` = `<state_dir>/digger.db`。我实跑确认守护进程的库在 `data/state/digger.db`。两个进程各用一个文件,"项目注册表只有这一份"这条总目标没达成。`digger_paths::projects_db_path()` 就是为此而设,但没有任何生产代码在用它。
- 测试漏网原因:`projects_db_path_equals_the_daemons_db_path` 比较的是 `product().db_path()` 与 `projects_db_path()`,二者同源,恒真;没有任何测试断言 GUI 实际打开的路径。

**修改要求:** `ProjectsState::open_in` 接收库路径与旧库路径(生产分别传 `projects_db_path()` 与 `legacy_projects_db_path()`),内部调 `ProjectHost::open_in`;补一个用生产入口(而非 `ProjectHost` 直接调用)的测试:预置旧 `projects.db` → 经 `ProjectsState` 打开 → 项目在 `digger.db` 且位置等于 `product().db_path()`、journal_mode 为 wal。

## B2(P1):迁移实现与计划要求不符

`crates/store/src/migrate.rs`:
1. **表结构硬编码了一份**(与 `dozer-store-project` 手抄),计划要求由该库建表。Dozer 一改 schema 就漂移,且旧库缺 `created_ms`、`description` 列时 `INSERT … SELECT` 直接失败。
2. **目标库已有数据一律返回 `AlreadyMigrated`**,旧文件不改名、冲突行不报错;计划测试 `ids_do_not_collide_when_digger_db_already_has_projects` 要求"不覆盖并返回错误"。现在用户若先启动过一次(哪怕只是 B1 修好前新建了项目),旧数据就被永久静默跳过。
3. **失败时会留下空的 `digger.db`**(迁移前先建库建表),与"要么不存在要么字节一致"不符;该测试(`a_failure_midway_…`)报告里没列出,我在 `migrate.rs` 的测试里也没找到。
4. `ATTACH DATABASE '{legacy_str}'` 用字符串拼路径,路径含单引号会 SQL 报错;应改为 `ATTACH DATABASE ?1`。
5. `memory_history` 的拷贝结果被 `let _ =` 丢弃,迁移计数不含它(可接受,但应在 outcome 或日志里体现)。
6. `set_wal_mode` 在库文件不存在时直接 `Ok`,而 `open_in` 里调用时 store 尚未建库,首次安装时 WAL 实际没设(与 B1 叠加)。应先让 store 建库再设,或让 `set_wal_mode` 自己创建。

## B3(P2):Task 9 / 11 的覆盖与测试质量

- `spawn_does_not_inherit_stdin` 只测 `find_diggerd` 的错误路径,名不副实;`ensure_returns_ok_when_a_daemon_already_answers`、`protocol_mismatch_is_reported_as_such` 两个计划要求的测试**没写**;"连点 Retry 只发起一次""启动结果送入 AgentShell 不再二次 ensure"也没有计数假 `ensure` 的测试。`product.rs` 只改了一条断言。
- `ensure` 里 `eprintln!` 违反 Digger 的日志约定(同其他 crate 用的日志设施);`projects.rs` 的 WAL 失败也是 `eprintln!`。
- `find_diggerd` 退回 PATH 用 shell `which`,GUI 启动环境的 PATH 很短,且多一个外部进程依赖;用 `std::env::split_paths` 自查更稳。
- 并发测试只做 2×50 次,计划写的是 2×1000 次交错、含记忆写入与 rename;且它没有走 `open_in`,所以没有真正验证 WAL 生效后的行为(`set_wal_mode` 是测试里手动调的)。
- `Cargo.lock` 已提交(+627 行,含本地 patch 解析结果),计划要求不提交。

## B4(P2):Task 11 Step 2 同机并存回归未做

报告自己承认没做,理由"复杂度"。计划明确要求命令行部分由实现者完成(隔离 `DIGGER_DATA_DIR` + 临时 `HOME`):socket/db/日志互不重叠、`digger-hook install claude` 与 `dozer-hook install claude` 条目并存、旧库升级、退出 GUI 后 `diggerd` 仍在。这些恰好是会暴露 B1 的验证。报告里的"Result: ✓"只是比较了两个 `ProductId` 字段,不是回归。另外报告测试数增量的算术("11+6+2+2 minus some overlap = 15")对不上,应列出真实新增测试名;测试名守恒也未做声明。

## 次要

- `diggerd` 日志文件名是 `dozerd.log`(来自共享库,Digger 身份下仍叫 dozerd),建议作为 Dozer 侧的跟进项上报,不在本期改。
- `dozer-hook` 识别过宽(`entry_is_product` 子串)的现状报告里没提,计划要求"若有则上报"。
- 打包脚本 `package-macos.sh` 的改动合理;`release.sh` 的校验段(Task 10 Step 2)没改,报告也没说明。
- ADR-015 已写;ADR-014 的 Consequences 指向与 `window-shell-design` 同步未见改动(Task 11 Step 3)。

## 下一步

按 B1、B2 修后复审;B3、B4 一并补齐。修复期间不要合并、不要打 tag。
