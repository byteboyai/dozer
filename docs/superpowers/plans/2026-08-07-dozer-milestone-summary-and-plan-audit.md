# Dozer 开发里程碑总结（截至 2026-08-07）

> 本文档不是实现计划,是一次全量审计后的**收尾报告**:核对 `docs/superpowers/plans/`
> 下全部计划是否都已落地,记录本轮审阅中发现并修复的问题,并对暂不处理的遗留项做
> 显式记录。写作目的是给"过去的开发任务"画一条清楚的分界线——从这份文档往后,
> 新的开发工作应该从新的 spec/plan 开始,不用再担心"是不是还有哪个旧计划没做完"。

## 一、审计范围与方法

对 `docs/superpowers/plans/` 下全部 29 份计划、`docs/superpowers/specs/` 下全部 34 份
spec 逐一核对:

- spec 侧:除去已废弃的 v0 spec、纯 spike 调研报告、各阶段"acceptance"验收记录(这些
  不是待实现的产出物),其余每份 spec 都能找到对应的实现计划,没有"有 spec 无 plan"
  的遗漏。
- plan 侧:每份计划挑出 2-4 个最有代表性的产出物签名(struct/enum/函数名),在当前
  代码里核实是否真实存在且被正常接线使用(不是死代码/stub/`todo!()`)。可疑或有出入
  的计划再深入核实到具体缺口。
- 对本轮新完成、风险最高的四份计划(Todo 面板、Git Log 面板转正、git2 数据层+文件树
  状态、预览编辑弹层)做了完整的 task-by-task 代码审阅(含 `cargo build/test/clippy/
  fmt` 实测),而不只是签名核对。

## 二、已实现的功能清单

### P1 系列(一期主线,2026-07-15 ~ 2026-07-27)

workspace/webview 骨架、dozerd session 核心、终端 GUI、预览面板(flyfish webview)、
agent 集成(OSC 扫描/agent 状态机)、验收闭环(Goal/acceptance/RecordAcceptance)、
项目层(ProjectStore/FileTree)、文件树 git 装饰、会话审阅、对话列表、外壳 chrome 保真度、
预览/终端 chrome 细节——12 份计划全部落地。其中 P1h(文件树 git 装饰)、P1i(会话审阅)、
P1j(对话列表)三份的具体接口名后来被更晚的外壳重构(shell-icon-rail、parallel-projects
等)取代或改名(如 `FileStatus` 演进成 `FileGitStatus`,`TabKind::Review` 演进成独立的
`RightView::Conversations` 面板),功能都在,只是签名跟计划原文不完全一致——这是预期
的自然演进,不是缺陷。

### 外壳与交互(2026-07-28 ~ 2026-07-31)

图标系统(`IconKind` + 18 个矢量图标)、项目树右键菜单、可拖拽调整的分栏布局
(后来被 `ShellLayout` 统一接管)、左右图标栏外壳(`shell-icon-rail`)、并行多项目
支持(页签/`WorkspaceSlot`/`with_project` 路由不变式)、外壳 chrome 样式配置化、
CodeBuddy/OpenCode 双 agent 适配器、多 agent 基础设施(`AgentKind`/协议扩展)——全部
落地。

**例外**:`shell-icon-rail` 计划里的"内容 pane 放大/还原"功能(`MaximizedPane`)已
实现,但 2026-08-06 的 `a0d324e` 提交**主动移除**了触发它的四个放大按钮(文件预览/
终端/会话审阅/浏览器各一个),提交说明原文写明"放大态相关状态保留,但当前已无 UI
入口可触发"——这是一次刻意的产品决策,不是本轮审计发现的缺陷,目前 `IconKind::
Maximize`/`MaximizedPane`/`maximize_overlay` 等相关代码是有意保留的死代码。是否要
彻底删除或重新接一个入口,留给后续产品决策,不在本轮处理范围。

### 近期功能(2026-08-04 ~ 2026-08-06)

工作区字号 token 化、CodeBuddy transcript 解析器、H0 项目中心(首页最近项目/对话)、
OpenCode 插件、Agent 域面板——全部落地且经核实是真实接线(非 stub)。

### 本轮重点审阅的四份计划(2026-08-06)

以下四份最初都被判定"已完成",但完整代码审阅发现了需要修复的具体问题,均已在本轮
会话中修复并通过 `cargo build/test/clippy/fmt` 验证:

**Todo 面板**(`.dozer/todo.md` 双态清单 + 筛选/搜索/派发/计划-完成时间)
- 修复:`Message::TabAttached` 里"派发到新建 tab"的补记逻辑误用了"当前聚焦项目"
  而非消息自带的 `project_id`,多项目并行场景下会把派发记录记错项目、或在切走页签
  后彻底丢失。
- 清理:5 处 clippy 违规(`needless_lifetimes`/`collapsible_if` ×2/`needless_update`/
  `unnecessary_min_or_max`)。

**Git Log 面板转正**(提交图分支/tag 标签、点选详情+diff、worktree 速览条、加载更多、
引用变化自动重建)
- 修复(Critical):选中提交算出的 diff 文本从未真正渲染到 UI,面板只显示文件列表。
- 修复(Critical):`git_log_cache` 是 `App` 级字段而非按项目存,切项目页签时面板
  停留在旧项目的提交图不刷新,与同面板里已经刷新的 worktree 速览条对不上——补了
  `sync_git_log_to_active_project` 统一在 `LeftIconSelect`/`ProjectTabSwitch` 两处调用。
- 修复(Critical):两处 `theme::GOLD`(甲方动作专属色)被挪用于纯状态展示,违反
  CLAUDE.md 硬性裁决。
- 修复(Important):worktree 速览条完全不可点击,违反计划"点击直接切换/打开项目
  页签"的要求——补了 `Message::ProjectTabOpen` 接线。
- 修复(Important):`Message::ProjectFsChanged` 的自动重建判断只看"当前聚焦项目路径
  是否匹配缓存",没看事件自带的 `project_id`,后台项目的引用变化会误触发前台项目的
  缓存重建、误清掉用户正在看的 diff。
- 补测:回填了计划里缺失的两个测试(根提交 diff、patch 文本非空断言)。

**git2 数据层 + 文件树状态增强**(delivery.rs 迁移到 git2、暂存/工作区双态、worktree
识别、`notify` 实时刷新)
- 修复(Critical):`Workspace::adopt_project`(新开项目页签的唯一路径)从未调用
  `start_git_watch`,D4 的实时刷新只对"跨重启恢复"的页签生效,对最常见的"直接开
  新项目"这条路径完全不生效,且无任何报错提示——这是这次审计里最隐蔽的一个缺口。
- 修复(Important):`git_watch::is_relevant_path` 只检查路径第一级目录名,monorepo/
  多包项目里嵌套的 `node_modules`/`target`/子模块 `.git` 不会被过滤,写依赖/编译产物
  时会触发无意义的刷新——改成检查路径全部层级。

**预览编辑弹层**(文件树内联文本编辑 + 保存/放弃确认)
- 修复(Critical):编辑弹层打开时,键盘事件(含回车)会同时被转发进背后的终端/
  agent 会话——默认布局(右侧终端展开)下几乎必现,是本轮发现的最严重的一个问题。
  补了 `App::edit_session_open()` 闸门 + Esc 关闭弹层的路由。

## 三、已知遗留、按你的决定暂不处理

以下问题在审计中被发现,但按你的决定**不在本轮修复**,留给后续人工使用中反馈时再处理:

1. **项目树右键菜单"从磁盘重新加载"未按 `is_dir` 收窄显示范围**
   (`crates/dozer-app/src/workspace.rs:8415-8419`)——在任意文件/目录的右键菜单里
   都会出现,而不是只在目录上。是否要收窄,取决于这个动作到底该理解成"重新加载
   这个目标"还是"重新加载整棵树"(它的实际实现是后者)。
2. **鼠标滚轮上报复用 `Message::TermInput`,继承了键盘输入的副作用**
   (`crates/dozer-app/src/term_view.rs:271`、`workspace.rs:3489-3496`)——alt-screen
   应用接收鼠标滚轮上报时,会顺带触发 `scroll_to_bottom()`/`selection_clear()`。
   alt-screen 下本地 scrollback 本就无意义,影响有限;如果用户选中了终端文本后再
   滚轮滚动一个吃鼠标事件的程序(如 vim/htop),选区会被意外清空。

这两项均来自更早一次 ad-hoc 审阅(`2026-08-05-pending-worktree-code-review.md`)、
在本轮之前就已存在且未修复,不是本轮新引入的问题。

## 四、当前工作树状态

以下三个文件的修复尚未提交(本轮会话中直接改的,还在 working tree 里):

```
M crates/dozer-app/src/git_log.rs
M crates/dozer-app/src/git_watch.rs
M crates/dozer-app/src/workspace.rs
```

对应"二、本轮重点审阅"里 Git Log 面板转正 + git2 数据层两份计划的全部修复项。已过
`cargo build -p dozer-app`/`cargo test -p dozer-app`(297 个测试全过)/`cargo clippy
--all-targets -- -D warnings`/`cargo fmt -- --check` 四项验证,可以随时提交。

## 五、结论

`docs/superpowers/plans/` 下 29 份计划、`docs/superpowers/specs/` 下 34 份 spec 已
全部核实完毕,没有"有计划无代码"的遗漏。本文档之前的全部开发任务在此画一条分界线:

- 功能层面:一期范围内规划的功能均已交付。
- 质量层面:本轮审阅中发现的 4 个 Critical 级问题(Todo 派发路由错项目、Git Log
  diff 未渲染、git watch 对新项目不生效、编辑弹层键盘泄漏进终端)均已修复并验证。
- 遗留项:两个已知的小问题(见"三")按你的决定留给后续人工反馈处理;"放大态"
  死代码(见"二")是刻意的产品决策,不是缺陷。

之后的开发工作请从新的 spec/plan 开始——不需要再回头假设"旧计划里可能还有没做完
的东西"。
