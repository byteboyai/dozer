# Dozer H0：项目中心落地页 + 顶栏 Dozer 页签化

> 状态：设计稿，要点经用户逐问确认（见下），待写完复审。
> 上游：规格 §3 需求列表 ⓪c“启动主界面·项目中心”（H0 帧）；Figma `Dozer Phase 1 UI` node-id=56:2。
> 动因（用户 2026-08-05）：现有 `home_page()` 只是品牌区+“打开项目…”+已开项目卡的占位实现，与规格/Figma 描述的 H0 落差很大；顶栏“Dozer”是个独立按钮，视觉/交互都没有并入页签行。这次把两者一起补齐。

## 0. 用户确认的关键决定

1. **页面范围 = 规格 §3(⓪c) 明确的一期切片**：左栏项目列表 + 右侧“最近的文件”“最近的对话”两卡。**不做**日历、社区教程墙——规格原文写"随后补"，Figma 里画出来了但不在这次范围内。
2. **“最近的文件”数据源 = 复用 git 改动 mtime**：不新增“最近打开/编辑文件”的全局持久化追踪；对最近活跃的几个项目分别跑现有的 `delivery::file_statuses`，取被 git 判定为改动/未跟踪的文件，读文件系统 mtime，合并倒序取前几条。语义是“最近有改动的文件”，不是“最近在 Dozer 里打开过的文件”——这次不做后者。

## 1. 目标

两件事，同一个入口串起来：

- 顶栏“Dozer”从一个独立按钮，改造成页签行里的**常驻页签**（视觉与项目页签一致：激活态 CARD 底 + BORDER 边框；不同的是它没有关闭按钮、不参与页签拥挤收窄，恒在最左）。
- 点它进入的落地页从现在的占位实现，换成规格 §3(⓪c) 描述的“项目中心”：左栏“我的项目”列表（含搜索框视觉占位）+ 右侧“最近的文件”“最近的对话”两张卡。

## 2. 关键裁决

- **D1 Dozer 页签＝复用 `project_tab_item` 的视觉语言，独立渲染函数**：新增 `dozer_home_tab(active: bool) -> Element`，结构=`icons::IconKind::Home` + `"Dozer"` 文字，套一层 `container` 应用与 `project_tab_item` 相同的 active/inactive 样式（CARD+BORDER / 透明），固定内容宽（`Length::Shrink`），高度吃满 `top_bar_height()`。`active = app.current_page == AppPage::Home`。`top_bar()` 里原来的 `title` 绑定改调这个函数，位置不变（仍在 `tabs` 左侧、`row![title, tabs, right]` 第一位）——视觉上它就是页签行的第一片页签，但不占用 `project_tabs_row` 的收窄计算，这是“恒在最左、不会被挤没”的合理行为，也是当前项目页签“＋”按钮同款的“固定位不参与收窄”处理。
- **D2 `App.recent_projects` 新增字段，只增不改**：`Workspace.recent_projects`（项目栏“未打开项目”兜底列表用）保留不动，避免为了这次改动牵连一条无关路径。新增 `App.recent_projects: Vec<ProjectInfo>`，在 `App::bootstrap()` 里 `list_projects()` 拿到 `known` 后顺手 `app.recent_projects = known.clone()`；`Message::ProjectTabOpened(project, recent)` 处理函数最上面加一行 `self.recent_projects = recent.clone()`，让每次开新项目/切最近项目之后这份列表也跟着刷新。H0 侧栏项目卡片直接读 `app.recent_projects`，取前 5 条（服务端 `list_projects()` 已按 `last_active_ms` 倒序，不需要客户端再排序）。
- **D3 点项目卡＝复用现成的 `Message::ProjectSelect(id)`**：不新增消息。这条已经处理“已开→前台化”“未开→当成新页签打开”两种情况（`focus_project_tab` + `ProjectTabOpen` 落地路径），H0 侧栏和项目栏“未打开项目”兜底列表用的是同一条消息、同一套语义,只是数据源不同（`app.recent_projects` vs `ws.recent_projects`）。
- **D4 “最近的文件”/“最近的对话”＝进入 Home 时异步加载一次，新增 `Message::HomeRecentsLoaded`**：`Message::TopBarHome` 处理函数在把 `current_page` 设成 `Home` 的同时，对 `self.recent_projects` 前 5 条 spawn 一个 `spawn_blocking` 任务：
  - 文件：对每个项目 `delivery::repo_root` 找到 repo → `delivery::file_statuses(repo)` 拿到改动/未跟踪文件路径集合 → `std::fs::metadata(path).modified()` 读 mtime → 跨项目合并按 mtime 倒序，取前 4 条（对齐 Figma 卡片行数）。
  - 对话：对每个项目 `conversation::list_all_conversations(cwd)`（P1j 已有，内部已聚合 Claude/CodeBuddy/OpenCode 三个 agent 来源并按 `modified_ms` 倒序）→ 跨项目再合并一次、倒序，取前 3 条。
  - 结果打包成 `Message::HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>)` 送回 UI 线程，写入 `App` 新增的两个字段 `home_recent_files`/`home_recent_conversations`。
  - **裁剪**：对话卡不做 Figma 里“进行中/已验收 vN”这一档状态字（需要跨项目交叉每个打开项目的存活会话 transcript_path + 验收记录，成本明显更高，且不在用户这次确认的范围内），只显示“标题 · 项目名 · agent · 相对时间”，与项目栏“未打开项目”场景已有的对话行文案精简版对齐。此项在本文档 §6 记一笔，不在这次范围内。
  - **新类型**（本地私有，定义在 `workspace.rs`）：
    ```rust
    struct HomeRecentFile { path: PathBuf, project_name: String, modified_ms: u64 }
    struct HomeRecentConversation { project_name: String, meta: ConversationMeta }
    ```
- **D5 相对时间文案＝复用已有逻辑**：`conversation_sub` 里“刚刚/N 分钟前/N 小时前/N 天前”那段判断已经是这次要的格式，抽出成独立的纯函数 `relative_time_text(modified_ms: u64, now_ms: u64) -> String`（`conversation_sub` 改为调用它），H0 的项目卡“活跃时间”、文件卡、对话卡三处都用它，不要三份重复switch。
- **D6 侧栏宽度＝新增几何 token，不写字面量**：`workspace.json` 的 `geometry` 节点新增 `h0_sidebar_width: 248`（设计基准，Figma 同值），`workspace_geometry.rs` 加 `pub fn h0_sidebar_width() -> f32 { GEOMETRY.h0_sidebar_width * icon_size::scale() }`，与本文件其余 chrome 尺寸同一套“JSON 基准 × 全局 scale”约定，不在 `home_page()` 里散落字面量。
- **D7 搜索框/“更多项目”＝视觉占位，不接线**：项目搜索输入框复用已有 `icons::IconKind::Search` 资源画放大镜图标，不做实际过滤（precedent：顶栏 ⌘K 搜索框已是同样的“视觉占位，检索排后续”处理）；“更多项目”按钮同样只还原视觉，不挂 `on_press`——它需要的“全部项目列表”视图现在不存在，属于后续增量（settings 齿轮已经是同款“先视觉后接线”的先例）。“＋新增项目”复用现有 `Message::ProjectTabPickFolder`（与项目栏那颗按钮同一入口）。

## 3. 组件与数据流

```
用户点顶栏 Dozer 页签
   │ Message::TopBarHome
   ▼
current_page = AppPage::Home
   │ spawn_blocking: 对 recent_projects.take(5) 跑
   │   delivery::file_statuses + fs mtime  → HomeRecentFile 列表
   │   conversation::list_all_conversations → HomeRecentConversation 列表
   ▼
Message::HomeRecentsLoaded(files, convs) → app.home_recent_files / home_recent_conversations
   ▼
home_page(app) 渲染：
  左栏 248px：logo+版本、"我的项目"、搜索框(占位)、
              app.recent_projects.take(5) 项目卡(点击→ProjectSelect(id))、
              "更多项目"(占位) + "＋新增项目"(→ProjectTabPickFolder)
  右侧：      "最近的文件"卡(home_recent_files)、"最近的对话"卡(home_recent_conversations)
```

- `crates/dozer-app/src/workspace.rs`：
  - `App` 新增字段 `recent_projects: Vec<ProjectInfo>`、`home_recent_files: Vec<HomeRecentFile>`、`home_recent_conversations: Vec<HomeRecentConversation>`；`new_shell` 里三者初始化为空。
  - `Message::HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>)`（新增变体）。
  - `Message::TopBarHome` 处理函数：置页 + spawn 上述任务。
  - `Message::ProjectTabOpened` 处理函数：顶部加一行同步 `self.recent_projects`。
  - `App::bootstrap()`：`known` 到手后同步一行给 `app.recent_projects`。
  - 新增私有类型 `HomeRecentFile`/`HomeRecentConversation`、私有函数 `relative_time_text`、`load_home_recents`（spawn_blocking 里跑的纯 IO 函数，签名 `fn load_home_recents(projects: &[ProjectInfo]) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>)`，可单测）。
  - `dozer_home_tab(active: bool) -> Element`（新增渲染函数）；`top_bar()` 改调它。
  - `home_page(app: &App) -> Element` 整体重写为 D1-D7 描述的两栏布局，替换现有实现；`project_tab_entries`/已开项目卡片那段旧逻辑随之删除（不再是 H0 的数据源，`ProjectSelect` 卡片改吃 `app.recent_projects`）。
- `crates/dozer-app/src/workspace_geometry.rs`：新增 `h0_sidebar_width()`。
- `crates/dozer-app/assets/theme/workspace.json`：`geometry.h0_sidebar_width = 248`。

## 4. 错误处理

- `app.recent_projects` 为空（全新安装、一个项目都没开过）：左栏只画“我的项目”标题 + 空状态提示文案（灰字“还没有项目”）+ 搜索框 + 两个按钮，不崩。
- `HomeRecentsLoaded` 还没回来（刚点进 Home 的那一帧）：两张卡先画“加载中…”占位文案，数据到了再替换——避免第一帧空白跳变。
- 某个 recent project 的路径已在磁盘上消失（用户手动删了目录）：`delivery::repo_root`/`file_statuses` 对不存在路径的行为已在 `delivery.rs` 现有实现里是"静默返回空"，这条项目在文件/对话两张卡里就是没有贡献,不特殊处理、不报错。
- `list_all_conversations` 对没有任何 agent 存储目录的项目返回空列表（P1j 已有行为），合并时自然被跳过。

## 5. 测试策略

Headless：
- `relative_time_text`：0/59/60/3599/3600/86399/86400 秒边界，对齐既有 `conversation_sub` 用例。
- `load_home_recents`：tempdir 造 2 个假项目（一个是 git repo 带改动文件+ mtime 可控、一个带假 Claude 对话目录），断言合并排序、跨项目、取前 N 的行为；空输入（`&[]`）→ 两个空 vec，不 panic。
- `dozer_home_tab`：现有 `project_tab_item` 类渲染函数没有 headless 单测先例（iced `Element` 不好断言），这次也不为渲染函数补——遵循既有约定（视觉靠人工验收，见下）。

人工验收（草案）：
1. 打开本仓 → 顶栏最左出现“Dozer”页签，样式与右侧项目页签一致（激活态描边+底色），当前在工作区视图时它是未激活态。
2. 点它 → 切到项目中心页：左栏 248px 显示项目 logo/版本、"我的项目"、搜索框、最多 5 张最近项目卡（名称/相对时间/路径/git 分支）、"更多项目"+"＋新增项目"两个按钮；右侧先短暂"加载中"，随后出现"最近的文件"（本仓最近 git 改动的文件，按时间倒序）与"最近的对话"（本仓 Claude 对话历史，按时间倒序）两张卡。
3. 点某张最近项目卡：若该项目已开着页签 → 直接切过去（不影响任何已有终端会话）；若没开 → 新开一个页签并前台化。
4. 点"＋新增项目" → 走现有文件夹选择流程，与顶栏"＋"/项目栏按钮行为一致。
5. 再点一次顶栏 Dozer 页签（此时已经在 Home）→ 数据重新拉一遍（体感：内容可能因磁盘变化而更新），不报错、不重复叠加。
6. 关掉所有项目页签，回到"一个项目都没打开"的状态 → Home 页仍然正常渲染（空列表兜底文案），不 panic。

## 6. 备选方案（已否/推后）

- **日历、社区教程墙**：Figma 画了，规格 §3(⓪c) 原文明确"随后补"，且社区板块指向外部站点（easyeasyai.com）需要单独的内容源，这次不做（用户 2026-08-05 确认）。
- **"最近的文件" = 全局"最近打开"持久化追踪**：更准确但要新增一条 `PreviewOpenPath` 时间戳落盘到 dozerd 的存储通路，一期成本明显更高；这次用 git 改动 mtime 顶上（用户 2026-08-05 确认，见 §0.2）。
- **对话卡显示"进行中/已验收 vN"状态字**：需要跨项目交叉每个打开项目的存活会话 transcript_path + dozerd 验收记录，成本高于这次范围；先只显示标题/项目/agent/相对时间，状态字留作后续增量（见 §2 D4 裁剪说明）。
- **把 Dozer 页签真正并入 `project_tabs_row` 的响应式收窄计算**：技术上可行，但会让"项目多到挤爆时 Dozer 页签本身被压窄甚至消失"，与它作为"恒定返回口"的产品意图矛盾；改为固定位、不参与收窄（见 D1）。
- **`App.recent_projects` 与 `Workspace.recent_projects` 合并成一份**：会牵连项目栏"未打开项目"兜底视图这条无关路径，且当前多项目并行架构下 `Workspace` 实例本就可能有多个，"唯一一份"反而要多想一层"该信哪个 Workspace 的" ——这次不做,两个字段并存(见 D2)。
