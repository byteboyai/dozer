# Dozer P1h 设计：文件树 git 装饰

> 状态：设计稿,用户已口头认可要点,待写 spec 后复审。
> 上游:承接 P1g 最小可用项目层（左一文件树 + 项目卡 git 分支脏）;规格 §7 左一"git 段常显当前分支与脏标记"。
> 起点:P1g 并入 main 后的工作区。

## 0. 范围

项目层后续拆四件独立事,dbx 组件视图已划二期(规格 §3)排除;本切片=**文件树 git 装饰**(用户 2026-07-19 选定,四件中最小、最高杠杆:放大 P1g 文件树、与验收闭环同源、`delivery::changes` 已有 porcelain 解析打底)。doctor / H0 独立页留后续。

## 1. 目标

在左一文件树的每一行按 git 状态上色 + 尾缀状态字符,让用户在树上一眼看出哪些文件改了/新增/删了;目录按 rollup 标记(含深层变更即标),折叠态也能看出"这里有改动"。

## 2. 关键裁决

- **D1 数据源 = `git status --porcelain`**:新增 `delivery::file_statuses(repo: &Path) -> HashMap<PathBuf, FileStatus>`,解析为"绝对路径 → 状态"(porcelain 给 repo 相对路径,拼 repo 根成绝对,与文件树行的绝对路径匹配)。`FileStatus` 三态:`Modified`(`M`/` M`/`MM`/`RM` 等含 M)、`New`(`??` 未跟踪 / `A` 暂存新增)、`Deleted`(`D`/` D`)。其余码归 `Modified`(保守)。
- **D2 上色沿用"金=有改动"约定**(P1g 项目卡 `main*` 脏标记已用金):`Modified`→金 `GOLD`、`New`→绿 `GREEN`、`Deleted`→红 `RED`、未变文件→现状 `CYAN`(文件)/`BODY`(目录)。每个带状态的行尾缀字符 `•`(改)/`+`(新)/`−`(删),不只靠颜色(可辨识性)。
- **D3 目录 rollup**:某目录(含深层)下有任一变更路径 → 目录行标金 `GOLD` + 尾缀 `•`。判定纯函数 `dir_has_change(dir, changed_paths) -> bool`(任一 changed 路径以 dir 为前缀)。渲染时对每个目录行查一次(O(目录行 × 变更数),典型规模无虞)。
- **D4 刷新时机复用 `ProjectGitRefreshed`**:把消息从 `(branch, dirty)` 扩成 `(branch, dirty, statuses)`,在打开项目(`ProjectOpened`)+ 每次回合结束(`AgentStateChanged(TurnEnded)` 的 git 检测)时一并算出 statuses 并回送。文件树状态机(P1g `FileTree`)不变——装饰是渲染层按 `statuses` 查表叠加,不进 `FileTree`。
- **D5 git 调用在 GUI 侧 spawn_blocking**:同 P1g/P1f,`file_statuses` 随 `branch`/`is_dirty` 一起在 `spawn_blocking` 里算,不阻塞 UI 线程;dozerd 不参与(哑管道)。

## 3. 组件与数据流

```
ProjectOpened / TurnEnded
   │ spawn_blocking: delivery::{branch, is_dirty, file_statuses}(repo)
   ▼
Message::ProjectGitRefreshed(branch, dirty, statuses)
   ▼
workspace 存 git_statuses: HashMap<PathBuf, FileStatus>
   ▼
project_pane 渲染文件树行:
   - 文件行:statuses.get(path) → 有则按状态上色 + 尾缀字符
   - 目录行:dir_has_change(path, statuses.keys()) → 有则金 + `•`
```

- `crates/dozer-app/src/delivery.rs`:加 `FileStatus` 枚举 + `file_statuses(repo)->HashMap<PathBuf,FileStatus>` + `dir_has_change(dir, &[PathBuf])->bool`(纯函数,便于测)。
- `crates/dozer-app/src/workspace.rs`:`Workspace` 加 `git_statuses: HashMap<PathBuf, FileStatus>`;`Message::ProjectGitRefreshed` 加第三参;git 刷新处一并算;`project_pane` 树行渲染查表上色 + 目录 rollup。
- 无新文件、无协议改动(纯 GUI 侧)。

## 4. 错误处理

- 非 git 项目:`file_statuses` 返回空 map(`git status` 失败),树全按未变色,不崩。
- porcelain 某行格式意外:跳过该行(不 panic),其余照常。
- 变更文件在树中不可见(未展开/在隐藏名单如 target):无对应树行,不影响(rollup 仍会让其祖先目录标记——若祖先可见)。

## 5. 测试策略

Headless:
- `file_statuses`:tempdir 真 git 仓库,改一个已跟踪文件、加一个未跟踪、删一个已跟踪 → 断言三个路径各自的 `FileStatus`;非 git 目录返回空。
- `dir_has_change`:给定变更路径集,断言含变更的目录返回 true、无关目录 false、深层变更也命中。
- `FileStatus` 从 porcelain 码的映射纯函数(若拆出):` M`→Modified、`??`→New、` D`→Deleted、`MM`→Modified。

人工验收(草案):打开本仓 → 改一个文件、加一个新文件 → 文件树对应行变金/绿 + 尾缀字符,其所在目录(折叠时)也标金;`git checkout`/提交后刷新(回合结束或重开项目)→ 装饰消失;非 git 目录无装饰不崩。

## 6. 备选方案(已否)

- **装饰进 `FileTree` 状态机**:让状态机持有 git 状态。否——文件树是"目录结构"的纯状态机,git 状态是正交的、刷新节奏不同的叠加层,放渲染层更清晰、`FileTree` 保持单一职责。
- **实时 inotify/FSEvents 刷新**:更即时但引入文件监听依赖与复杂度;回合结束 + 打开项目触发对 dogfooding 足够,留后续。
- **staged/unstaged 细分 + ±行数**:验收 tab 已有 ±行数;树上细分暂不必要(YAGNI),都归 Modified。
