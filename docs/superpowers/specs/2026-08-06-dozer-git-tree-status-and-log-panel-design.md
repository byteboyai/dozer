# Dozer 设计：文件树 git 状态增强 + Git Log 面板

> 状态：设计稿,已与用户逐节确认,待写完整 spec 后走 writing-plans。
> 上游:承接 P1g 最小可用项目层(git 分支/脏标记)、P1h 文件树 git 装饰(三态色点+尾缀字符,`docs/superpowers/specs/2026-07-19-dozer-p1h-filetree-git-decoration-design.md`)、2026-08-06 gleisbau spike(`crates/dozer-app/src/git_log.rs`,已验证对本仓真实历史——含合并提交 `2429d15`——布局正确);规格 §7 左一"git 段常显当前分支与脏标记"、§4 技术选型"gitoxide 读路径用 gix"。
> 起点:main 现状(spike 已并入 `crates/dozer-app/src/git_log.rs` + 左图标栏第三个按钮,尚未走完整设计评审)。

## 0. 范围

两件事共享同一份数据层重构,合并一次设计:

1. **文件树 git 状态增强**:P1h 的三态色点(金改/绿新/红删)升级为暂存/工作区双态感知 + 实时刷新(不再只在开项目/回合结束时刷新) + 同仓库多 worktree 感知(项目模型认识 `git worktree list`,不再假设 1 项目=1 工作目录)。
2. **Git Log 面板**:spike 转正——提交图(列/颜色/合并线)、分支/tag 标签、选中提交的改动文件列表+diff、面板顶部 worktree 速览条(点击用现有 P2a 项目页签机制打开)。位置沿用 spike:左图标栏第三个面板(Files/Web 同级),不是项目栏底部"文件|git|组件"切换条(那条维持现状,不接线,见 §6)。

**范围内的操作:只读**。不做暂存/提交/checkout/merge/rebase——Dozer 的定位是"agent 提交、人验收",git 面板是给人看清 agent 做了什么用的审查工具,不是要在 Dozer 里重建一个完整 git 客户端。

## 1. 目标

- 用户在文件树上不仅能看出"这个文件改了",还能看出"是暂存了还是只在工作区";改动发生后(不管是 Dozer 里 agent 改的还是用户在外部编辑器/终端改的)树上的装饰能在毫秒到亚秒级跟上,不用等下一次开项目或回合结束。
- 用户能在 Dozer 里看到同一个仓库当前还有哪些其他 worktree(哪个分支、干不干净),不用切到外部终端敲 `git worktree list`;点一下就能把那个 worktree 当新项目页签打开。
- 用户能在 Dozer 里像用 JetBrains Git 工具窗一样"看提交图"：列/颜色区分分支脉络、合并提交清楚可辨、选中一个提交能看到它具体改了哪些文件、每个文件的 diff——但仅限于看,不提供任何会改写仓库状态的操作入口。

## 2. 关键裁决

- **D1 数据源迁移到 `git2`,推翻规格 §4"gix 读路径"**:`delivery::branch`/`is_dirty`/`file_statuses` 目前 shell 出 `git status --porcelain` 文本解析。`git2` 已经因为 spike 引入的 `gleisbau`(依赖 `git2 = "0.21"`)成为传递依赖,直接把这三个函数迁到 `git2::Repository` API 上,理由:①避免维护第二套 git 后端(gix)只为了跟 gleisbau 用的 git2 并存;②`git2::Repository::statuses()` 原生区分 `INDEX_*`(暂存)/`WT_*`(工作区)标志位,不用再猜 porcelain 双字符码的语义;③`git2::Repository::worktrees()`/`find_worktree()`/`open_from_worktree()` 直接给 worktree 列表,不用再拼一套 `git worktree list --porcelain` 解析器;④进程内调用,不为每次刷新 fork 一个 `git` 子进程——D4 的实时刷新一旦刷新频率上去,这个差距会被放大。写路径(`accept()` 的 `git update-ref`)维持 shell 出 `git` CLI 不变,不在本设计范围内。
- **D2 `FileStatus` 升级为暂存/工作区双态**:

  ```rust
  pub enum ChangeKind { New, Modified, Deleted }
  pub struct FileGitStatus {
      pub kind: ChangeKind,
      pub staged: bool,   // INDEX_* 非空,即 index 与 HEAD 不同
      pub unstaged: bool, // WT_* 非空,即工作区与 index 不同
  }
  pub fn file_statuses(repo: &Path) -> HashMap<PathBuf, FileGitStatus>
  ```

  `kind` 判定沿用现有优先级(含 `WT_NEW`/`INDEX_NEW` → New;含 `WT_DELETED`/`INDEX_DELETED` → Deleted;其余 → Modified),`staged`/`unstaged` 可同时为真(部分暂存 + 又有新改动,对应旧 porcelain 的 `MM`)。一个文件只要出现在返回的 map 里,`staged`/`unstaged` 至少一个为真。

  渲染规则(扩展现有"色点+尾缀字符"语言,不引入 JetBrains 的文件名染色约定——那套约定 P1h 时已经隐式否决过一次):**色点填充态**编码暂存(`staged && !unstaged` → 实心;`unstaged`(不论是否同时 `staged`)→ 空心环),**色点颜色**继续编码 `kind`(金/绿/红不变),**尾缀字符**继续编码 `kind`(`•`/`+`/`−`不变,不重复编码暂存态——填充/空心已经够用,再叠一层字符是过度设计)。

  目录 rollup(`dir_status`)同步升级成 `DirGitStatus { kind, staged, unstaged }`:`kind` 取子孙中最"重"的(Deleted/Modified > New,与现状一致);`staged`/`unstaged` 只要子孙任一为真就为真。

- **D3 worktree 列表**:

  ```rust
  pub struct WorktreeInfo {
      pub name: String,
      pub path: PathBuf,
      pub branch: Option<String>,
      pub dirty: bool,
      pub is_current: bool,
      /// worktree 元数据还在(.git/worktrees/<name> 存在)但工作目录已被删/
      /// 移走(常见于用户手动 rm -rf 而不是 `git worktree remove`)。
      pub missing: bool,
  }
  pub fn worktrees(repo: &Path) -> Vec<WorktreeInfo>
  ```

  `Repository::worktrees()` 只列出**链接**worktree,不含主工作树,需要单独探测当前 `repo` 是否为主工作树(`repo.commondir() == repo.path()` 时是主工作树)并把它作为第一项手工插入(`name` 用仓库目录名,`is_current` 按 `repo` 参数与各 worktree 路径的 canonical 比较得出)。每个 worktree 的 `branch`/`dirty` 通过 `Repository::open_from_worktree(&wt)` 开出该 worktree 自己的 `Repository` 视图后复用 D1 的 `branch`/`is_dirty` 逻辑取得;若 `open_from_worktree`/路径访问失败(目录已删),`branch=None`、`dirty=false`、`missing=true`,不整项跳过(§4 错误处理呼应)。

- **D4 实时刷新走 `notify`(规格 §4 已 accepted,首次真正使用)**:项目打开时启动一个 debounced(~300ms)文件系统监听,监听范围 = 仓库根,跳过 `project::HIDDEN`(`.git`、`target`、`node_modules`、`.DS_Store`,与文件树懒加载共用同一份名单,`project.rs:15`)之外的目录,**额外**显式监听 `.git/HEAD`、`.git/index`、`.git/refs/**`、`.git/packed-refs`(即便这些理论上在 `.git` 排除名单里,也要单独订阅,因为分支切换/外部提交/其他 worktree 提交都靠这几个文件的变化感知)。debounce 到期后复用现有 `spawn_project_git_refresh` 的 `spawn_blocking` 管线(见 §3),不新开一条刷新路径;若这次触发源包含 `.git` 引用类文件,额外触发一次 Git Log 快照重建(D5)。监听器随项目关闭/切换而停止(挂在 `Workspace` 生命周期上,现有的"项目切换清理"逻辑里加一步)。
- **D5 Git Log 面板转正**:沿用 spike 的位置(左图标栏第三个面板)与直线连线渲染(不做贝塞尔曲线打磨,YAGNI——已在上次对话里定为非目标)。在 spike 基础上新增:

  - **分支/tag 标签**:gleisbau 的 `GitGraph.labels: LabelMap` 在 spike 里被算出来又丢弃了(`git_log.rs` 目前只取 `graph.tracks`/`graph.layout`,没碰 `graph.labels`)。`CommitRow` 加一个字段:

    ```rust
    pub struct RefLabel { pub name: String, pub kind: RefKind }
    pub enum RefKind { LocalBranch, RemoteBranch, Tag }
    // CommitRow 新增:
    pub refs: Vec<RefLabel>,
    ```

    构建时对每个 commit 调 `graph.labels.get_labels(commit.oid)`,把 `gleisbau::print::label::Label { name, kind, .. }` 映射到 `RefLabel`(`LabelType::LocalBranch/RemoteBranch/Tag` 一一对应)。渲染:commit 摘要文字前加彩色 pill(本地分支用 `theme::GOLD`、远程分支 `theme::CYAN`、tag `theme::GREEN`,当前 HEAD 所在分支额外加一圈描边区分)。
  - **提交详情子面板**:选中一行(新增 `Message::GitLogSelectCommit(git2::Oid)`,选中态记入 `App.git_log_selected: Option<git2::Oid>`,与 `git_log_cache` 同生命周期、随 `repo_path` 变化一起清空)后,面板下半区(复用现有 pane 内部上下分割的既有模式,如 `divider_bar` 那套)展示该提交改动的文件列表(`git2::Repository::diff_tree_to_tree(parent_tree, commit_tree, None)` 逐文件),点文件展开该文件的 diff 文本(`Patch::from_diff` 或逐 hunk 渲染,纯文本着色即可,不需要语法高亮——语法高亮属于既有 Flyfish 预览引擎的职责,不在这里重做)。合并提交(≥2 parent)按第一父做 diff(与 `git show` 默认行为一致),不做三方 diff。
  - **worktree 速览条**:面板顶部(标题行下方)加一条横向列表,内容 = D3 的 `worktrees()` 结果,每项显示 `name · branch{*if dirty}`,`missing` 的项置灰不可点。点击:若该路径已经是某个打开的项目页签,聚焦那个页签;否则复用现有"新增项目"路径(P2a)打开为新页签——不新造一套"次级工作区"概念。
  - **分页**:抛弃 spike 里固定的 `MAX_COMMITS = 200`。面板底部滚动到底时,把 `Builder::with_max_count` 的参数从当前值加大一档(比如 +200)重新跑一次 `build()`,整体替换缓存快照。gleisbau 的 API 是"从头按 `max_count` 走一遍 revwalk 出整份布局",没有增量/游标接口,想要"追加"就只能整份重算——`with_max_count` 加大后重算一次,对几千提交量级的仓库仍是毫秒到低两位数毫秒级(revwalk 本身的成本,不是布局算法的成本),可以接受。
  - **缓存失效**:`App.git_log_cache` 现按 `repo_path` 失效(spike 已有)。新增:D4 的 `.git` 引用变化也触发失效重建(不再要求用户手动切换面板才刷新)。

## 3. 组件与数据流

```
notify watcher(debounced, 项目打开时起、关闭/切换时停)──┐
ProjectOpened / TurnEnded ─────────────────────────────┼──▶ spawn_blocking:
                                                        │      delivery::branch/is_dirty/file_statuses/worktrees(repo)
                                                        │      + 若触发源含 .git 引用变化: git_log::build(repo)
                                                        ▼
                              Message::ProjectGitRefreshed(project_id, branch, dirty, statuses, worktrees)
                              Message::GitLogRefreshed(project_id, Result<GitLogSnapshot, String>)
                                                        ▼
        Workspace.git_statuses: HashMap<PathBuf, FileGitStatus>  |  Workspace.worktrees: Vec<WorktreeInfo>
        App.git_log_cache: Option<GitLogSnapshot>
                                                        ▼
        project_pane 树行(填充/空心色点 + 尾缀字符)     |  git_log::view(worktree 速览条 + 提交图 + 详情子面板)
```

- `crates/dozer-app/src/delivery.rs`:`branch`/`is_dirty`/`file_statuses` 从 shell 出 `git` CLI 改为 `git2` 调用(D1);`FileStatus` 拆成 `ChangeKind` + `FileGitStatus`(D2);`dir_status` 返回类型同步升级为 `DirGitStatus`;新增 `worktrees(repo) -> Vec<WorktreeInfo>`(D3)。
- `crates/dozer-app/src/git_watch.rs`(新文件):封装 `notify` watcher 的启停(`start(repo, debounce, on_change)`/`Drop` 时自动 unwatch),不直接碰 `Message`——回调交给调用方决定发什么消息,保持这个模块本身可 headless 测(用临时目录+真实文件系统事件断言 debounce 合并、`HIDDEN` 目录被跳过)。
- `crates/dozer-app/src/workspace.rs`:`Workspace` 加 `worktrees: Vec<WorktreeInfo>` 字段与对应 watcher 句柄;`Message::ProjectGitRefreshed` 的 `statuses` 参数类型跟着 D2 升级,新增 `worktrees` 参数;新增 `Message::GitLogRefreshed(project_id: i64, result: Result<git_log::GitLogSnapshot, String>)`,由 D4 的 watcher 在 `.git` 引用变化时触发的 `spawn_blocking` 任务在完成后经 `proxy.send_event` 送达,处理方式与现有 `git_log_cache`/`git_log_error` 赋值逻辑(见 spike 的 `LeftIconSelect` 分支)一致,只是触发源从"用户点开面板"变成"watcher 检测到引用变化",两处共用同一段构建+赋值代码(抽成 `fn refresh_git_log(&mut self, repo_path: &Path)` 私有方法,避免重复);新增 `Message::GitLogSelectCommit(git2::Oid)`;项目切换/关闭清理逻辑里加"停掉这个项目的 watcher"一步;`tree_row_dot` 一类的树行渲染函数按 D2 的填充/空心规则改写;`left_panel_area` 的 `LeftView::GitLog` 分支不变(仍委托 `git_log::view`),但要把新的 worktree 列表/选中提交态传进去。
- `crates/dozer-app/src/git_log.rs`:`CommitRow` 加 `refs: Vec<RefLabel>`(D5);新增提交详情获取函数(基于 `git2::Repository::diff_tree_to_tree`);`view()` 签名扩展,接收 worktree 列表 + 当前选中提交,内部拆成"图(上)+详情(下)"两块,复用 `divider_bar` 一类既有分割组件;去掉 `MAX_COMMITS` 常量,`build()` 参数化 `max_count`。
- 无协议改动(dozerd 不参与,纯 GUI 侧——同 P1h D5 的既有裁决)。

## 4. 错误处理

- 非 git 目录:所有新函数(`file_statuses`/`worktrees`/`git_log::build`)返回空集合/空快照,树/面板不装饰、不崩,与 P1h 现状一致。
- `git2` 调用失败(仓库损坏、权限问题等):按函数返回类型的"空/None"分支处理,不 `unwrap`/`expect` 到 panic(测试代码里的 `expect` 除外)。
- worktree 目录已被外部删除(`missing: true`):列表里保留该项(不是直接从 `worktrees()` 结果里消失,因为 git 元数据还认为它存在),置灰、点击不可用,而不是让用户以为这个 worktree 从没存在过。
- `notify` 监听启动失败(比如 fd 耗尽等系统级限制):静默降级为"只在开项目/回合结束时刷新"(D4 之前的行为),记一条 `tracing::warn`,不影响项目正常打开。
- 提交详情 diff 计算失败(二进制文件、极大 diff 等):对应文件条目显示"无法生成 diff"文案,不影响文件列表里其余条目正常展开。

## 5. 测试策略

Headless(真实临时目录 git 仓库,复用 `delivery.rs`/`git_log.rs` 现有测试基础设施):

- `file_statuses`:暂存一个已跟踪文件的部分改动、再叠加工作区改动 → 断言该路径 `staged=true, unstaged=true`;纯暂存新增 → `staged=true, unstaged=false`;纯工作区改动未 `git add` → `staged=false, unstaged=true`。
- `dir_status` rollup:混合暂存/未暂存子文件 → 断言目录级 `staged`/`unstaged` 均正确按"任一子孙为真"聚合。
- `worktrees`:`git worktree add` 出 2 个额外 worktree(含一个未提交改动的)→ 断言列表含主工作树 + 2 个链接 worktree,`is_current`/`dirty`/`branch` 各自正确;删掉其中一个 worktree 目录但不 `git worktree remove` → 断言对应项 `missing=true` 而不是从列表消失。
- `git_watch`:tempdir 起 watcher,连续快速写多个文件 → 断言 debounce 只触发一次回调(不是每次写入都触发);写 `HIDDEN` 名单内目录下的文件 → 断言不触发;写 `.git/HEAD`(模拟外部 `git checkout`)→ 断言触发。
- `git_log::build` 扩展现有 `build_against_real_repo` 测试:断言已知分支尖(如当前 `main`)对应的 `CommitRow.refs` 包含一个 `RefKind::LocalBranch { name: "main" }`;`with_max_count` 从小到大调用两次 → 断言第二次结果行数增加且前 N 行与第一次结果一致(重算稳定性)。

人工验收(草案):打开本仓 → 暂存一个改动 + 再叠加一处未暂存改动,树上对应文件从空心变实心再变空心(叠加改动后);不经 Dozer、在外部终端 `git commit` 一次,几百毫秒内文件树装饰与 Git Log 面板的提交图同时更新,不用手动切页签/重开项目;点 Git Log 面板顶部 worktree 速览条的另一个 worktree → 作为新项目页签打开;选中一个历史提交(含至少一个合并提交)→ 详情区正确列出改动文件与 diff。

## 6. 备选方案(已否)

- **继续 shell 出 `git` CLI,只扩展 porcelain 解析**:改动面最小,但 worktree 列表还要再拼一套 `git worktree list --porcelain` 解析器,且高频实时刷新下反复 fork 子进程比 `git2` 进程内调用贵。用户已确认接受 D1 的迁移成本,否决本选项。
- **按规格 §4 原定,状态/worktree 走 `gix`,只有 Git Log 图走 `git2`**:保留原有技术选型决定,但导致仓库同时挂两套 git 后端,总依赖面积比 D1 更大,且没有实质好处(`git2` 已经因为 `gleisbau` 进了依赖树)。用户已确认接受推翻原决定,否决本选项。
- **Git Log 面板挂在项目栏底部"文件|git|组件"切换条,而不是左图标栏新面板**:更贴合规格原始设想的信息架构(窄栏~250px),但用户明确选择了左图标栏新面板(与 spike 一致、可展开空间更大)。项目栏底部那条"git {分支}"文案维持现状(纯文本,不接线、不改动),不在本次范围内一并处理——它已经在显示分支名,只是不可点,后续如果要统一入口再单独立项处理这处不一致。
- **文件树装饰改用 JetBrains 的"文件名染色"约定(蓝=改/绿=新/红冲突等)**:与现有色点+尾缀字符的视觉语言不是一套体系,P1h 定稿时已隐式否决过一次(见该设计文档 D2);本次只做暂存态的填充/空心扩展,不推翻既有配色约定。

## 7. 非目标 / 后续

- 暂存/提交/checkout/merge/rebase 等任何会改写仓库状态的操作入口——本设计通篇只读。
- 提交图的贝塞尔曲线/精细排版打磨——spike 的直线连线已经能正确表达分支拓扑,视觉打磨留后续按需再做。
- Blame 视图、文件历史(JetBrains "Show History for Selection" 一类)——不在"类似 JetBrains Git Log"的核心范围(提交图+详情)内,后续如有需要单独立项。
- 项目栏底部"文件|git|组件"切换条的接线/一致性清理——见 §6,不在本次范围。
- 超大仓库(数万提交/数千文件)的性能上限——测试策略覆盖的是正确性,不是这个规模下的性能基准;如果 dogfooding 中遇到明显卡顿再针对性优化。
