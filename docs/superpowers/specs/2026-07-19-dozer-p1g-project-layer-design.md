# Dozer P1g 设计：最小可用项目层

> 状态：设计稿,待用户审阅。范围裁决因用户暂离由实施方按"最小可逆 + 用户已表达意图"代拟,**逐条可推翻**。
> 上游:规格 §7(左一项目栏:项目卡 + 文件树/git/组件 + 分支脏标记 + doctor)、H0(项目中心);承接 P1f 验收闭环。
> 动因(用户 2026-07-19):"有了项目树,我就可以真的在 Dozer 里进行开发了"——项目层是把已建的终端/预览/验收串成可用整体的脊柱。
> 起点:P1f 并入 main 后的工作区。

## 0. 范围裁决(代拟,可推翻)

**项目 = 组织原则(选项 A),但每个触点做最小。** 选定的"当前项目"重锚三处:新终端 tab 默认开在项目根、验收闭环挂到项目仓库、文件树=项目的树。理由:贴合用户"真开发 + 验收挂项目"的诉求;§7 把项目栏放最左最主,空占位是最显眼的缺。

**做**:项目对象(打开文件夹/git 仓库、持久化、最近项目、当前项目)｜左一项目卡(名称/路径/git 分支+脏)｜懒加载文件树(点文件进预览)｜新终端 tab 开在项目根｜验收/交付检测挂到当前项目仓库。
**不做(后续)**:H0 项目中心独立页(用左一的"打开/最近"下拉替代)｜组件视图(dbx,二期)｜doctor｜文件树的 git 装饰(改动标记)/重命名/新建/删除文件｜.gitignore 精确过滤(先用固定隐藏名单)｜多项目并列(单当前项目)。

## 1. 目标

1. **项目对象**:打开一个文件夹为项目(git 仓库优先,普通目录也可),持久化到 dozerd,记最近项目与当前项目,重开 app 恢复当前项目。
2. **左一项目栏**:项目卡(名称 + 路径 + git 分支/脏标记 `main*`)+ 懒加载文件树(目录展开/收起,点文件 → 左二预览)。
3. **重锚**:新终端 tab 默认 cwd = 当前项目根(无项目时回落 `$HOME`);验收/交付检测的仓库 = 当前项目(无项目时回落会话 cwd,即 P1f 现状)。
4. **切换**:项目栏顶部"打开项目…"(rfd 文件夹选择) + 最近项目列表切换。

## 2. 关键裁决(代拟,可推翻)

- **D1 项目存 dozerd SQLite**:与验收记录同库同权威(dozerd 是状态真相源;CLAUDE.md 职责表:dozerd 持验收闭环存储)。两表:`projects(id INTEGER PK, path TEXT UNIQUE, name TEXT, last_active_ms INTEGER)` + `meta(key TEXT PK, value TEXT)`,当前项目 = `meta['active_project_id']`(可扩展存其它单例设置)。协议增 `ListProjects / OpenProject{path} / SetActiveProject{id} / GetActiveProject`。
- **D2 文件树在 GUI 侧读文件系统**:懒加载,展开某目录时才 `read_dir`(spawn_blocking,不阻塞 UI,同 delivery 的 git 调用)。dozerd 不碰项目文件树(哑管道原则延续)。固定隐藏名单:`.git`、`target`、`node_modules`、`.DS_Store`(.gitignore 精确过滤留后续)。
- **D3 git 状态走 git CLI**:复用/扩展 `delivery.rs`——加 `branch(repo)->Option<String>`(`rev-parse --abbrev-ref HEAD`)、`is_dirty` 已有。项目卡显示 `分支名` + 脏时缀 `*`。刷新时机:打开项目时 + 每次交付检测后(TurnEnded)顺带刷新。
- **D4 当前项目重锚 = 最小改动**:
  - `spawn_new_tab` 的 cwd:当前项目根 if set,else `$HOME`(现状)。
  - 交付检测/验收装载:仓库 = 当前项目路径 if set,else `tab.effective_cwd()`(P1f 现状)。即在 `AgentStateChanged(TurnEnded)`/`AcceptanceOpen` 里,优先用当前项目仓库。
- **D5 单当前项目**:同一时刻一个当前项目(左一显示它)。切换靠"打开/最近"。多项目并列(如竞品的项目侧栏)留后续。
- **D6 启动恢复**:app 启动 `bootstrap` 时向 dozerd 拉当前项目,装进 workspace;无则项目栏显示"打开项目…"空态。

## 3. 组件与数据流

```
rfd 文件夹选择 / 最近项目点击
   │ OpenProject{path} / SetActiveProject{id}
   ▼
dozerd projects.rs(SQLite):存/取/置当前 → Reply::Projects / Reply::Project
   │
   ▼(client 拉取)
workspace: 当前项目(Project{path,name}) + FileTree 状态机
   ├─ 项目卡:name + path + branch/dirty(delivery::branch/is_dirty,spawn_blocking)
   ├─ 文件树:展开目录 read_dir(spawn_blocking)→ TreeNode;点文件 → Message::PreviewOpenPath
   ├─ 新终端 tab:cwd = 项目根
   └─ 验收/交付检测:repo = 当前项目路径
```

- `crates/dozer-core/src/protocol.rs`:`ProjectInfo{id,path,name,last_active_ms}` + `Request::{ListProjects, OpenProject{path}, SetActiveProject{id}, GetActiveProject}` + 对应 `Reply::{Projects{...}, Project{Option<ProjectInfo>}}`。
- `crates/dozerd/src/projects.rs`(新):rusqlite,`ProjectStore::{open, upsert_and_activate(path)->ProjectInfo, list()->Vec, active()->Option, set_active(id)}`。表 `projects` + 当前项目标识。
- `crates/dozer-app/src/project.rs`(新):`FileTree` 状态机(纯数据:根路径、展开集、节点缓存;`toggle(path)`、`visible_rows()->Vec<TreeRow{path,depth,is_dir,expanded}>`)。不碰 iced。
- `crates/dozer-app/src/delivery.rs`:加 `branch(repo)->Option<String>`。
- `crates/dozer-app/src/workspace.rs`:当前项目字段 + 项目卡/文件树渲染(左一)+ 重锚接线 + 新消息。
- `crates/dozer-client/src/lib.rs`:项目相关方法。

## 4. UI(左一项目栏,最小)

- 顶部:项目名(CREAM)+ git `分支*`(脏时金`*`);副行灰色路径。无项目时:"打开项目…"按钮 + 最近项目列表。
- "打开项目…"(rfd 文件夹选择,main.rs 侧执行,同 PreviewPickFile 模式)。
- 文件树:缩进行,目录前 `▸/▾` 可点展开收起,文件点击 → 左二预览;固定隐藏名单过滤。
- 底部"最近项目"折叠区(切换)。doctor/组件视图留占位。

## 5. 错误处理

- 选的目录不存在/不可读:项目栏红字域内提示,不设当前项目。
- 非 git 目录:仍可作项目(文件树可用),git 分支/脏区显示"—"(不是仓库)。
- 文件树某目录 read_dir 失败(权限):该节点显示"(无法读取)",不崩树。
- dozerd 项目落库失败:红字提示,GUI 仍以内存态可用(下次重启丢失,可接受)。
- 当前项目被外部删除:打开时 read_dir 失败 → 提示"项目路径已失效",回空态。

## 6. 测试策略

Headless:
- projects.rs:upsert 幂等(同 path 不重复)、list 按 last_active 排序、set_active/active 往返、持久化(重开库仍在)——tempdir SQLite。
- protocol:ProjectInfo/消息 roundtrip。
- project.rs(FileTree):tempdir 建目录树,toggle 展开/收起,visible_rows 顺序与缩进,隐藏名单过滤,空目录/不可读目录不崩——纯函数 + tempfile。
- delivery::branch:tempdir git 仓库,返回当前分支名;非 git 返回 None。
- workspace:重锚纯函数(`effective_project_repo(active, session_cwd)->PathBuf`)——当前项目优先、回落会话 cwd。

人工验收清单(草案):打开本仓为项目 → 左一见项目名 + `main*` + 文件树;点 `README`/某 .rs → 左二预览;新终端 tab 落在项目根;`.dozer/goal.md` 定标后让 claude 改文件 → 回合毕横幅(验收挂到项目仓库);切到另一个 git 仓库项目 → 一切跟随;关 app 重开 → 当前项目恢复。

## 7. 备选方案(已否)

- **项目只做侧边文件面板(不重锚)**:体量更小,但不兑现"验收挂项目 / 在 Dozer 里开发",与用户诉求不符。
- **项目存 config.toml 而非 SQLite**:文件态简单,但当前项目/最近项目是结构性状态,与验收记录同属 dozerd 权威更一致;且 dozerd 已有 SQLite。
- **一步做全 H0 项目中心页 + 组件视图 + doctor**:体量过大,违背薄片;用左一"打开/最近"替代 H0,组件/doctor 后置。
