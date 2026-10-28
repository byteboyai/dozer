# S7/S8 收尾之后的剩余事项计划

日期:2026-10-28。承接 `2026-10-27-s7-s8-closeout.md` §7。本计划只列**没完成**的事,逐项给出做法、验收和顺序。已完成的不重复。

## 0. 事项总览

| # | 事项 | 类型 | 需要用户 | 改哪里 | 建议顺序 |
|---|------|------|----------|--------|----------|
| R1 | ~~D4 风险:已存为「收起」的群聊布局无入口展开~~ **已完成** | 核实+修复 | 无 | Digger | 1 |
| R2 | 新行为的人工 GUI 验收 | 验收 | 是 | 无 | 2(与 R1 同一次) |
| R3 | Digger 没有远端,提交只在本地 | 配置 | 给仓库地址 | Digger | 随时,越早越好 |
| R4 | ~~Codebuddy 群聊发言~~ **已完成**(Dozer `53a7100a`,tag `libs-2026-10-28b`;Digger 已重钉) | 验证+小改 | 无 | Dozer `group_adapter.rs` | 3 |
| R5 | ~~G1 搜索框右键菜单~~ **已完成(整框语义,待人工验证)**(Digger `2a23894`) | 新功能 | 中文输入法人工验证 | Digger | 4 |
| R6 | Opencode 群聊发言 | 阻塞于上游 | 无 | Dozer | 搁置,见触发条件 |
| R7 | X1 跨仓库迁移(**阶段 A 已完成**,阶段 B 未开始) | 跨仓库 | 排期 | byteui、bytehost、双方重钉 | 单独计划 |
| R8 | 零碎:门禁脚本重新 vendor、Dozer 项目面板用失败明细 | 可选 | 否 | Digger / Dozer | 有空再做 |

依赖:R1 的结论决定要不要追加修复;R4 需要新的 libs tag(与下一次库改动合并发,不为它单发);R5、R7 互不依赖也不挡其它项。

## R1 D4 风险核实 —— 已完成(2026-10-28,Digger `shell.rs`)

**结论:** 风险属实——布局文件的 `panes` 持久化每个面板的 `collapsed`,且 `ShellMsg::TogglePane` 是唯一把它置位的入口。已按推荐的 A 方案修复:读盘前 JSON 转换 `group_chat_never_collapsed`,保留分栏比例,不写盘,带失败测试先行。下面的勾选项保留作记录。

**问题:** `list_collapse_button` 与展开条删掉后,`ShellMsg::TogglePane(group_chat_id())` 在群聊里不再有发送方。若 Digger 的布局存储(`bytehost_desktop::PaneStore`)把「群聊已收起」持久化,这样的用户重启后看到收起的群聊且无法展开。

- [ ] **读代码定性(不需要用户):** 确认 `PaneStore`/布局是否落盘(看 `crates/desktop/src/shell.rs` 的布局保存、`bytehost_desktop` 里 `PaneStore` 的序列化)。结论写进本节。
- [ ] **若不持久化:** 本项关闭,只在 open-items 加一句「收起态不持久」。
- [ ] **若持久化:** 二选一,写失败测试先行——
  - A. 加载布局时,群聊面板的 `collapsed` 一律按 `false` 处理(最小,与 Dozer「群聊无收起」一致);
  - B. 在 `product.rs` 的 `pane_collapsed(&group_chat_id())` 处强制 `false`,并在 `group_chat_native_rect` 去掉 `collapsed` 分支。
  推荐 A:源头修,不留死分支。测试:构造一份带 `group_chat: collapsed` 的布局,加载后 `pane_collapsed` 为 `false`。
- [ ] 门禁:Digger 干净快照整套(`git archive`,路径不含 digger),`check-host-deps.sh`/`check-shared-deps.sh`/`check-shell-regions.sh`,fmt/clippy 零新增。

## R2 人工 GUI 验收清单(用户做)

用 `/Applications` 之外的开发构建,不动正在运行的应用。逐条点,结果记到 `digger/docs/superpowers/reviews/2026-10-28-s7-s8-closeout-acceptance.md`:

| 项 | 操作 | 期望 |
|----|------|------|
| 弹窗文案 | 打开「新建群聊」 | 常驻说明为「目前仅 Claude、Codex、Qodercn 可以发言」 |
| 无按钮条 | 群聊列表顶部、聊天区顶部 | 没有收起/展开按钮,聊天区顶部没有多余空白条 |
| 新建群失败 | 断开 `diggerd` 后建群 | 弹窗与主题恢复,错误提示出现;再输入新主题不被旧草稿覆盖 |
| 导入失败提示 | 在 home 的 agent 目录放一份损坏的 `.jsonl`,点「导入历史会话」 | 提示条出现「有 N 个会话文件导入失败…」并列出文件名与原因 |
| 删项目清副本 | 有会话的项目,删除(任一范围) | 会话列表中该项目的会话消失,**home 下的原 transcript 文件仍在** |
| Qodercn 发言 | 建群拉入 Qodercn,`@` 它 | 能回复,不改动项目文件 |

## R3 Digger 远端

- [ ] 用户给出仓库地址 → `git remote add origin <url>`,`git push -u origin main`。
- [ ] 推送前看 `git log origin/main..main`(首次推送则是整个历史),确认不含不该公开的内容;**工作区里 13 份已删除的 S6 文档仍是未提交状态,不要被一并提交**。
- [ ] 推送后 `check-shared-deps.sh` 里「本地领先 origin」类提示应消失;记入记忆索引。

## R4 Codebuddy 群聊发言 —— 已完成(2026-10-28)

**结果:** 登录后金丝雀实测通过,已实现并经真实 runner 冒烟 5/5;`libs-2026-10-28b` 已推送,Digger 同日重钉、弹窗说明已加入 Codebuddy。下面的勾选项保留作记录。

**阻塞:** 调研时 CodeBuddy 未登录(401),没有只读的行为证据。参数面与 Claude 同构(`--tools`、`--allowedTools`/`--disallowedTools`、`--no-session-persistence`)。

- [ ] 用户登录 CodeBuddy(`codebuddy` 交互登录一次)。
- [ ] 复用调研方法(`docs/superpowers/specs/2026-10-27-group-chat-agents-readonly-research.md` §方法):隔离目录、金丝雀 `canary.txt`、依次尝试创建文件/shell 写文件/读金丝雀/联网。判据:读成功,其余被拒,不出现 `pwned*`。
- [ ] 先写失败测试:`build_turn_command` 的 Codebuddy 分支参数断言(白名单只含 `Read,Grep,Glob`,逐个 `--disallowedTools`,`--no-session-persistence`,提示词走 stdin;**不得**出现 `-y`/`--dangerously-skip-permissions`)。同时把 `agents_without_a_verified_readonly_mode_stay_unsupported` 里的 Codebuddy 去掉。
- [ ] 仿 `real_qodercn_is_readonly_through_the_runner` 加 `#[ignore]` 手动冒烟 `real_codebuddy_is_readonly_through_the_runner`,**走真实 `HeadlessGroupRunner`**,通过两次。
- [ ] 注意 Codebuddy 的 transcript Dozer 是摄取的:确认 `--no-session-persistence` 后群聊发言**不会**混进用户的会话列表(查 `~/.codebuddy/projects/` 下是否新增)。
- [ ] 若验证不通过:结论写进调研记录,保持 `Unsupported`,不放宽。
- [ ] 通过后:更新调研记录、Digger 弹窗说明加入 Codebuddy、`CREATABLE_AGENTS` 与可发言集合保持一致。随下一次 libs tag 发出,Digger 同日重钉。

## R5 G1 搜索框右键菜单 —— 已完成(2026-10-28,待人工验证)

**结果(Digger `2a23894`):** 用户选定「整框语义」。会话搜索框右键弹 `byteui::native_menu`:剪切/复制作用于整个搜索词(剪切后清空),粘贴追加到末尾(多行折成空格),全选精确;搜索词为空时剪切/复制置灰。仅 macOS。**没有按计划照搬 Dozer 的做法:** Digger 是标准 `iced::application`,没有能合成键盘事件回灌的自定义事件循环,且 iced 0.14 的输入框操作读不到选区。搜索词由 `ConversationsPanel` 在 `SearchInput` 经过时缓存(库的 `search_draft` 是 crate 私有)。要做到按选区的菜单,需要在 byteui 输入框上加「读选区」(新 byteui 版本、双方重钉),未做。

**待人工验证(R2 清单追加):** 右键搜索框 → 菜单弹在光标处;输入词后「复制」再粘贴到别处一致;「剪切」清空搜索框;「粘贴」追加;「全选」后输入会替换;**中文输入法:拼音候选未提交时右键,候选不丢**;⌘C/V/X/A 快捷键仍是按选区。

下面的勾选项是原计划,保留作记录。

**背景:** `TextInputMenuOpen` 在 Digger 是空操作(`product.rs` 的 `PM::TextInputMenuOpen(..) => {}`、files 的 `F::TextInputMenuOpen` 被忽略,会话搜索框同理)。Dozer 的做法:`byteui::native_menu::show(text_input_menu_items(..), last_right_click)` 弹原生菜单,选中后把动作映射成 ⌘X/C/V/A(`menu_edit_key`),记入 `pending_native_menu_edit_key`,下一帧由 `window_events` 先聚焦目标输入框再合成键盘事件回灌。

- [ ] **先设计后动手(很可能需要 brainstorming):** Digger 的事件循环是否有等价于 Dozer `window_events` 的"每帧后处理"位置;`last_right_click` 坐标从哪取(Digger 已有 `native_menu::show` 用法:`conversation_filter_items`、`tab_overflow_items`,可看其 `at` 的来源)。
- [ ] 范围先限定**会话搜索框**(S7b 报告里点名的),Project/Files/Todo 的输入框另议。
- [ ] 菜单项:剪切/复制/粘贴/全选;密码类输入锁剪切复制(`secure`)。i18n 沿用 Digger 现有方式。
- [ ] 测试:菜单项生成(含 `secure` 变体)与 `menu_edit_key` 映射可单测;「合成键盘事件作用到正确输入框」需要 iced 运行态,做成 gallery 示例 + 人工验证。
- [ ] **中文输入法人工验证一次:** 拼音候选未提交时右键不丢候选;选「粘贴」后文本进输入框、不进 Agent 终端。
- [ ] 门禁同 R1。

## R6 Opencode(搁置)

阻塞原因(见调研记录):无限制工具参数;`webfetch: deny` 不能挡住联网;无「不落会话」开关;提示词只能走 argv(受命令行长度限制)。

**重新评估的触发条件(缺一不可):** 上游提供能禁联网且实测生效的机制;能不落会话库;提示词有非 argv 通道。到时再按 R4 的流程走一遍。在此之前弹窗说明保持如实,不为凑齐而放宽。

## R7 X1 跨仓库迁移 —— 计划已写

计划:`docs/superpowers/plans/2026-10-28-x1-cross-repo-migration.md`(阶段 A byteui 页签/菜单,阶段 B bytehost-desktop 可信原生子 webview 执行器;先做 Task 1 盘点)。下面是写计划之前的原始要点。

页签部件(`agent_tabs.rs`)、菜单弹层(`menu_overlay.rs`)、`review_native`/`native_host` 的 wry 样板迁入 byteui / bytehost。**单独写计划**,不并入任何库 tag。动手前先确认:bytehost 稳定窗口;Dozer 的 `native_menu` 重新导出是否已覆盖菜单弹层的非 mac 回退(该回退「非 mac 从未实际编译过」,只要求能编译);两个仓库各自发版、Dozer 与 Digger 同日重钉。

## R8 零碎(可选)

- [ ] Digger 的 `check-shared-deps.sh` 是 vendor 自 bytehost v0.4.4,现为 v0.4.12:重新 vendor(`tools/shared-deps-guard`),跑通后提交。
- [ ] Dozer 项目面板的「导入历史」也可显示失败明细(`backfill_project_transcripts_report` 已就绪);这会改变 Dozer 用户可见行为,先问用户再做。
- [ ] 偶发测试 `apps::tests::closing_during_startup_waits_and_releases_the_root`、`adapters::agent_hooks::tests` 仍是「无复现、未定性」;若再出现,在并行高负载下整套反复跑来定性,不要靠「重跑到绿」。

## 约束(沿用)

- 独立 worktree/分支;不碰 Dozer 主工作区用户未提交的 `Cargo.lock` 与 `crates/dozer-app/web/review-trace/`;不碰 Digger 里 13 份已删除的 S6 文档、`feat/preview-pairing`、`digger-p4`、`/private/tmp/digger-sandbox`、`/Applications` 里的运行应用。
- 提交带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`;`git diff --cached --stat` 看全貌再提交。
- 库改动合成一个 libs tag(沿用 `libs-YYYY-MM-DD`),发之前跑 `check-shared-deps.sh`、`check-log-scope.sh`、`check-i18n.sh`、全 workspace 测试;Digger 同日重钉并重跑整套门禁。
- 所有门禁在干净快照里跑;flaky 测试用重复隔离运行量化。
- 群聊发言必须执行层只读,**严禁** `--dangerously-skip-permissions`/`-y`;没有验证到只读机制的 CLI 保持 `Unsupported`。
- 推送、打 tag 等对外动作先得到用户确认。
