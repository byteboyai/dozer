# 新建项目对话框设计

## 背景与动机

现状"打开项目"其实只是选一个已存在的文件夹(`rfd::FileDialog::pick_folder()`
→ `Message::ProjectTabOpen(dir)` → `client.open_project(&path)`)。三处入口
(topbar "+"菜单、H0 首页"＋新增项目"、Project 面板空态)全部复用这一条路径,
2026-09-17 甚至把按钮文案从"新建项目"改回"打开项目",因为当时它确实不能
新建——只能打开已有的。

这次要补的是真正的"新建":在本地新建一个空项目目录,或者签出一个 Git 远程
仓库到本地作为新项目。产品参考见三张线框图(本地新建 tab、URL 签出
tab、账户仓库列表 tab)。

本设计只覆盖 URL 签出这一种远程接入方式;线框图里 GitHub/GitLab/Gitee
账户接入(OAuth/token、仓库列表拉取)明确推迟,详见"非目标"。

**长期方向**(本次讨论中用户明确给出的指令,记在此处供后续弹窗设计参考):
以后新增的弹窗都应该能够盖住 wry 预览 webview,不能像旧路径那样依赖手动
`set_visible(false)` 隐藏 webview。本设计采用的独立窗口机制正是这个方向
的落地方式之一。

## 目标 / 非目标

**目标:**

1. topbar "+" 按钮菜单在现有"打开项目"之后新增"创建项目"入口,打开一个
   两 tab 的"新建项目"对话框。
2. Tab 1「新建本地项目」:选根目录、填项目名称/描述、可选是否顺带创建
   Git 仓库,提交后在本地新建一个空项目目录并注册为 Dozer 项目。
3. Tab 2「签出Git远程仓库的项目」:仅实现"仓库URL"一种接入方式——填远程
   仓库 URL、目标根目录、项目名称(默认从 URL 推导)、描述,提交后
   `git clone` 到本地并注册为 Dozer 项目。
4. 两条路径共用同一套"目标路径校验→落盘→注册"收尾逻辑,复用现有
   `client.open_project`/`project_tab_opened` 而不是另起一套项目注册机制。
5. 修正一个在设计过程中发现的既有行为缺陷:现有的静默 scaffold
   (`project::spawn_scaffold_run`)对每个新开的项目 tab 无条件
   `git init`(见"对既有共享代码的修正"一节),导致新建本地项目时"创建
   Git仓库"复选框取消勾选也不起作用——需要让这个复选框真正生效。

**非目标:**

- GitHub/GitLab/Gitee 账户登录接入(OAuth/PAT、仓库列表拉取)——线框图
  中对应的三个侧边栏入口本次只做禁用占位,悬浮提示"敬请期待"。
- 任何形式的 Git 凭证输入 UI 或凭证存储(不新增 keyring 命名空间)。克隆
  鉴权完全委托系统已配置的 SSH agent / macOS Git Credential Manager /
  `~/.netrc`。
- 完整的系统环境检测/安装向导("doctor"面板)。本次只在这个对话框内做一次
  轻量 `git --version` 检测 + 内联提示,不新建独立的系统设置页面——完整
  doctor 面板是 legacy-boy 尚未迁移进 dozerd 的那块工作范围(见
  `[[dozer-legacy-boy-deletion-blocked]]`),等那块工作启动时再收编这里的
  检测逻辑,不重复建设。
- 克隆进度的百分比解析(不解析 `git clone --progress` 的输出,只用"转圈+
  按钮禁用"表达"进行中")。
- 非 macOS 平台的独立窗口回退路径(仓库当前 mac 先发,遵循既有原则)。

## UI 设计

### 入口

`crates/dozer-app/src/chrome/topbar.rs` 的 `project_add_menu_spec`(现有
"+"按钮弹出菜单,结构为:最近未开项目列表 → 分隔线 → "打开项目")在分隔线
下方追加第二项"创建项目",紧跟在"打开项目"之后。H0 首页"＋新增项目"按钮
与 Project 面板空态入口本次不改动,仍指向"打开项目"那条旧路径——是否也
加"创建项目"入口留给后续小改动按需处理,不在本次范围内展开。

### 对话框整体

标题"新建项目",两个 tab:「新建本地项目」/「签出Git远程仓库的项目」,
与三张线框图一致。

### Tab 1:新建本地项目

字段(自上而下):

- **根目录**:文本框 + 文件夹图标按钮,点击弹出 `rfd::FileDialog::
  pick_folder()`。
- **项目名称**:单行文本框。
- **项目描述**:多行文本框。
- **创建Git仓库**:复选框,默认勾选(与线框图一致)。
- **创建项目**按钮(主操作)。

提交时最终项目路径 = `根目录/项目名称`。

### Tab 2:签出Git远程仓库的项目

左侧竖排四项:「仓库URL」(可用) / 「GitHub」/ 「GitLab」/ 「Gitee」(三项
禁用置灰,悬浮提示"敬请期待",不可点击)。

选中「仓库URL」时右侧字段:

- **远程仓库**:单行文本框,填 clone URL(https 或 ssh 均可,取决于用户
  自己填什么、系统 git 能不能处理——Dozer 不做协议校验)。
- **根目录**:同 Tab 1,文件夹选择。
- **项目名称**:默认从 URL 最后一段路径推导(去掉 `.git` 后缀),可编辑。
- **项目描述**:多行文本框。
- **签出项目**按钮(主操作)。

提交时最终项目路径同样是 `根目录/项目名称`,即 clone 的目标目录。

### 渲染宿主与生命周期

对话框渲染在一扇独立原生窗口里,复用
`feature/overlay-window-shared-abstraction` 分支已抽出的
`platform/overlay_gpu.rs::OverlayGpu` / `platform/overlay_window.rs::
open_child_window`,新增消费方 `platform/project_create_overlay.rs::
ProjectCreateOverlay`(持有 `window`/`gpu: OverlayGpu`,方法
`open`/`redraw`/`handle_input`/`reposition`,结构上与已有的
`FileHistoryOverlay` 同构)。

与 `SearchOverlay`/`FileHistoryOverlay` 的一处关键差异:**不接入
`FocusTracker`,不做"失焦即关闭"**。原因是这个表单含多字段输入,且"根
目录"字段要触发嵌套的 `rfd::FileDialog::pick_folder()`——那是另一扇原生
面板,弹出时会让本窗口短暂失焦,若照搬失焦关闭逻辑会在用户选目录的
过程中把整个表单连同已填内容一起误关掉。这个窗口只认 **Esc 键** 或
**显式"取消"按钮** 关闭。IME 需要开启(`window.set_ime_allowed(true)`,
`overlay_gpu.rs` 抽取时留的教训 3 明确要求消费方自己做这一步)。

互斥机制:新增 `OverlayKind::ProjectCreate` 变体,加入
`close_other_overlays` 的判断——打开这个对话框时,若 search/file_history
有一个开着,先关掉它;反之亦然。

## 架构

新增 `crate::project_create` 模块(状态/消息组织方式参照
`extensions/file_history.rs`,但不挂在 `extensions/` 目录下,因为这是全局
对话框而非项目内面板):

```rust
pub struct State {
    pub active_tab: Tab,          // Local | Clone
    pub local: LocalForm,         // 根目录/名称/描述/是否创建git仓库
    pub clone: CloneForm,         // URL/根目录/名称/描述
    pub error: Option<String>,    // 内联错误区(路径已存在/git未检测到/clone失败等)
    pub busy: bool,               // 提交后置真,按钮禁用+转圈,直到成功/失败回调
}

pub enum Message {
    TabSelected(Tab),
    LocalRootDirPick,              // 触发 rfd 异步任务
    LocalRootDirPicked(Option<PathBuf>),
    LocalFieldChanged(LocalField, String),
    LocalGitCheckboxToggled(bool),
    CloneRootDirPick,
    CloneRootDirPicked(Option<PathBuf>),
    CloneFieldChanged(CloneField, String),
    SubmitLocal,
    SubmitClone,
    CloneFinished(Result<PathBuf, String>),  // git clone 子进程结果
    Cancel,
}
```

## 数据流

### Tab 1:新建本地项目 提交流程

1. 校验:`根目录/项目名称` 拼出目标路径,若已存在(不论空目录还是非空)
   直接在内联错误区报错拦下——创建只认全新目录,"已存在则复用"是"打开
   项目"该管的语义,两条路径不混淆。项目名称同时做文件系统非法字符
   校验(空字符串、含 `/`、首尾空白均拒绝)。
2. `std::fs::create_dir_all(目标路径)`。
3. `project_meta::write_description(目标路径, 描述文本)`(即使描述为空也
   写,保持行为一致,不做"空则不写"的特殊分支)。
4. 调用 `client.open_project(&目标路径)` → 成功后与现有"打开项目"完全
   相同地触发 `Message::ProjectTabOpened(project, recent, skip_git_init)`,
   其中 `skip_git_init` = 复选框**未勾选**时为 `true`,否则 `false`(见下
   一节的共享代码修正)。
5. 关闭本对话框窗口,新项目以新 tab 打开(复用 `project_tab_opened` 现有
   的开 tab 逻辑)。

### Tab 2:签出Git远程仓库的项目 提交流程

1. 校验:`根目录/项目名称` 目标路径同样要求不存在;URL 非空校验(不做
   协议/格式深度校验,交给 `git clone` 自己报错)。
2. 轻量 git 环境检测:执行 `git --version`(`std::process::Command`,同步
   起个 `spawn_blocking`),失败(找不到可执行文件)则内联报错提示"未检测
   到系统 git,请先安装 Xcode Command Line Tools"并给出可复制的安装命令
   (`xcode-select --install`),不继续。
3. 检测通过后,异步 `spawn_blocking` 执行 `git clone <url> <目标路径>`
   (`std::process::Command::new("git").arg("clone")...`),按钮切换为
   "签出中…"+禁用+转圈。
4. 失败:将子进程 stderr 原样展示在内联错误区,不解析、不重试,允许用户
   修改字段后再次提交。
5. 成功:与本地创建 Tab 相同——`project_meta::write_description` 写描述、
   `client.open_project`、`ProjectTabOpened(.., skip_git_init: false)`
   (克隆下来的仓库天然带 `.git`,现有 `ensure_git_repo` 步骤本来就会
   `AlreadyOk` 跳过,不需要特殊处理)、关闭对话框、开新 tab。

## 对既有共享代码的修正

设计过程中发现:`app/update.rs::project_tab_opened` 对每个新开的项目 tab
都会调用 `project::spawn_scaffold_run`(`extensions/project/update.rs:220`),
其内部 `scaffold::run_sync_steps` 无条件跑 `ensure_git_repo`——即当前
**任何**新开的项目 tab,只要本地没有 `.git` 就会被自动 `git init`,不受
本设计新增的"创建Git仓库"复选框控制。若不修正,复选框取消勾选将没有
实际效果。

修正方式:给共享函数加一个默认为 `false` 的显式开关,不影响其余调用方
行为:

- `Message::ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>, bool)`
  新增第三个字段 `skip_git_init`。
- `project_tab_opened(&mut self, project, recent, skip_git_init: bool)`
  把 `skip_git_init` 一路传给 `spawn_scaffold_run(.., skip_git_init)`。
- `spawn_scaffold_run` 新增 `skip_git_init: bool` 参数,内部改调
  `scaffold::run_sync_steps(repo, skip_git_init)`。
- `scaffold::run_sync_steps(repo: &Path, skip_git_init: bool)` 遍历
  `scaffold_steps()` 时,若 `skip_git_init` 为真则跳过标签为"git 仓库"
  的那一步(不改 `ScaffoldStep`/`ScaffoldStepResult` 类型本身,只在遍历
  处加一个按 label 的判断)。
- `spawn_repair_run`("修复项目"按钮的完整跑法)不受影响,不加这个参数
  ——"修复"场景本来就应该无条件把 git 仓库补全,不应该受一次性创建时的
  复选框选择影响。
- 现有唯一调用点(`app/update.rs:1853`,即"打开项目"这条旧路径)传
  `false`,行为与今天完全一致。

## 校验与错误处理

| 场景 | 处理 |
|---|---|
| 目标路径(`根目录/项目名称`)已存在 | 提交前内联报错拦下,不落盘、不发起 clone |
| 项目名称含 `/` 或为空/纯空白 | 提交前内联报错拦下 |
| 根目录未选择 | 提交按钮保持禁用(两个 tab 一致) |
| URL 签出:系统未检测到 git | 提交前内联报错 + 安装命令提示,不发起 clone |
| URL 签出:`git clone` 进程返回非 0 | 内联展示 stderr 原文,允许改字段重试 |
| dozerd `open_project` 调用失败 | 复用现有 `project_tab_opened` 的失败分支(`daemon_error` 提示),对话框已经关闭的情况下退化为顶层错误条,不在对话框内二次展示 |

## 测试策略

- 纯函数单测:目标路径拼接与"已存在即拒绝"判断、项目名称合法性校验、
  从 URL 推导默认项目名称(`https://github.com/abc/foo.git` → `foo`,
  `git@gitlab.com:abc/bar` → `bar` 等常见形态)。
- `scaffold::run_sync_steps(repo, skip_git_init)` 补一个
  `skip_git_init = true` 时"git 仓库"步骤不出现在返回列表里的单测,原有
  `skip_git_init = false` 行为保持现有测试覆盖不变。
- git 副作用类操作(`git clone`/`git init`/真实网络请求)没有自动化测试
  手段,走人工 `cargo run` 验证清单,覆盖:
  - topbar "+"菜单"打开项目"/"创建项目"两项均可用,顺序正确。
  - 本地创建:勾选/不勾选"创建Git仓库"分别验证目标目录最终有/没有
    `.git`。
  - URL 签出:克隆公开仓库成功;克隆私有仓库(本机已配置好凭证)成功;
    克隆私有仓库(未配置凭证)失败时 stderr 展示;目标目录已存在时报错
    拦截。
  - 未安装系统 git 时(临时改 `PATH` 模拟)URL 签出 tab 给出检测失败
    提示。
  - 对话框打开时若 search/file_history 开着会被自动关闭,反之亦然。
  - 根目录 `rfd` 文件夹选择器弹出/关闭过程中,创建项目对话框本身不会
    被误关闭,已填字段保留。
  - Esc 键与"取消"按钮均可关闭对话框且不留副作用。
