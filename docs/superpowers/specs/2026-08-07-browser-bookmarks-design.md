# 浏览器收藏夹设计

**状态：已批准（brainstorming 会话，2026-08-07）**

## 背景

浏览器域(`crates/dozer-app/src/preview.rs` 的 `PreviewPane`，被 `Workspace::browser` 独立持有，
详见其文档注释)目前只有 tab 列表 + 地址栏编辑态，网址用完即走，没有持久化的"常用地址"概念。
用户提出要加收藏夹功能：分"全局"（跨项目共享）和"本项目"（只在当前项目里可见）两级，地址栏
新增一个按钮把当前网址加入其中之一。

现有代码已有两条持久化路径可参考：

- `crates/dozerd`（daemon 侧结构化数据）走 SQLite，`dozer.db`（`dozer_core::paths::state_dir()`
  下），`ProjectStore`/`AcceptanceStore` 各持一个连接，写频度低不需要连接池。
- `crates/dozer-app`（GUI 本机状态）走纯 JSON 文件（如 `open_projects.json`）。

收藏夹的"本项目"范围需要跟"项目"这个概念对齐——`ProjectInfo{id, path, name, last_active_ms}`
已经由 `dozerd::ProjectStore` 按 `path` upsert 持久化，`id` 天然稳定（同 path 复用同 id）。
收藏夹选 dozerd SQLite 这条路，新表通过外键复用 `projects.id`，不必再自己维护一份项目身份。

## 目标 / 非目标

**目标**：
1. 新增 dozerd 侧 `bookmarks` 表：全局收藏 + 项目收藏两种 scope，项目收藏挂靠
   `projects.id`。
2. 协议新增 `AddBookmark`/`RemoveBookmark`/`ListBookmarks`，`ListBookmarks{project_id}` 一次
   拉回"全局 + 该项目"的合集。
3. 地址栏新增星标按钮：当前网址已被收藏（全局或本项目任一）则实心 GOLD，否则空心 DIM；点击
   弹出小菜单选择"加入/移出全局收藏"、"加入/移出本项目收藏"。
4. tab 栏新增"收藏夹"按钮，点击展开下拉面板，分"全局"/"本项目"两组列出条目，点条目新开
   tab 打开，每条带 `×` 删除按钮。
5. 收藏标题直接复用当前 tab 标题（`PreviewTab.title`，即 `open_url` 里已经算好的
   host 部分），不引入额外输入步骤。
6. 同一 URL 允许同时存在于全局和本项目收藏夹——两边互相独立，互不影响。

**非目标**：
- **不做拖拽排序/文件夹分组** —— 列表按 `created_ms` 顺序展示，YAGNI。
- **不做编辑标题** —— 标题固定复用加入时的 tab 标题；想改名先删再加。
- **不做导入导出、多设备同步** —— 数据只在本机 `dozer.db`，不涉及云同步。
- **不做收藏夹内搜索/筛选** —— 面板条目量级预期很小（个人常用网址），不需要。
- **不给收藏夹本身发消息通知/角标** —— 纯本地增删查功能。

## 关键语义确认（brainstorming 会话定案）

- 存储选 dozerd SQLite，不选 dozer-app 本地 JSON——理由：多开 GUI 实例/未来多客户端场景下
  能共享同一份收藏，且与项目列表用同一套持久化机制，认知负担最小。
- 星标按钮点击 → 弹出小菜单选"全局/本项目"，不是两个并排的独立按钮——省一个图标位，交互
  路径跟"删除项目收藏"复用同一个入口。
- 查看入口是地址栏旁的下拉面板，不是常驻收藏栏——不占用垂直空间，跟现有 tab
  栏/地址栏紧凑布局的风格一致。
- 收藏标题固定复用当前 tab 标题，不引入命名弹窗。
- 全局和本项目视为两个完全独立的集合，同一 URL 可以同时存在两边；星标的"已收藏"实心状态是
  "任一边存在即真"，不做互斥迁移。

## 架构与数据流

### 1. `dozerd::bookmarks`：SQLite 表与 `BookmarkStore`

新增模块，结构镜像 `projects.rs` 的 `ProjectStore`：

```sql
CREATE TABLE IF NOT EXISTS bookmarks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope TEXT NOT NULL CHECK(scope IN ('global', 'project')),
    project_id INTEGER,              -- scope='global' 时必须为 NULL
    url TEXT NOT NULL,
    title TEXT NOT NULL,
    created_ms INTEGER NOT NULL
);
-- 全局按 url 去重；项目按 (project_id, url) 去重。
-- 用两条局部唯一索引而非单条表级 UNIQUE：SQLite 的 UNIQUE 约束把多个
-- NULL 视为互不相同，project_id 为 NULL 时不能指望它去重全局收藏。
CREATE UNIQUE INDEX IF NOT EXISTS ux_bookmarks_global
    ON bookmarks(url) WHERE scope = 'global';
CREATE UNIQUE INDEX IF NOT EXISTS ux_bookmarks_project
    ON bookmarks(project_id, url) WHERE scope = 'project';
```

```rust
pub struct BookmarkInfo {
    pub id: i64,
    pub scope: BookmarkScope,       // Global | Project
    pub project_id: Option<i64>,
    pub url: String,
    pub title: String,
    pub created_ms: u64,
}

impl BookmarkStore {
    pub fn new(path: &Path) -> Result<Self>;
    /// INSERT OR IGNORE 语义：同 (scope, project_id, url) 已存在则 no-op，
    /// 不更新标题（见"非目标"）。
    pub fn add(&self, scope: BookmarkScope, project_id: Option<i64>, url: &str, title: &str)
        -> Result<BookmarkInfo>;
    pub fn remove(&self, id: i64) -> Result<()>;
    /// 全局收藏 ∪（project_id 给定时）该项目收藏，按 created_ms 升序。
    pub fn list(&self, project_id: Option<i64>) -> Result<Vec<BookmarkInfo>>;
}
```

`dozerd/src/main.rs` 里与 `ProjectStore`/`AcceptanceStore` 同样指向
`state_dir().join("dozer.db")`，同一个 SQLite 文件里多一张表，不需要新连接管理逻辑。

### 2. 协议扩展（`dozer-core::protocol`）

```rust
// Request
AddBookmark { scope: BookmarkScope, project_id: Option<i64>, url: String, title: String },
RemoveBookmark { id: i64 },
ListBookmarks { project_id: Option<i64> },

// Reply
Bookmarks { bookmarks: Vec<BookmarkInfo> },
```

`server.rs` 里的分发逻辑照抄 `OpenProject`/`ListProjects` 的写法，调用 `BookmarkStore` 对应
方法，`AddBookmark`/`RemoveBookmark` 成功后回 `Reply::Ok`（同 `RecordAcceptance` 的既有模式），
失败回 `Reply::Error`。

### 3. `dozer-app` 状态

`Workspace` 新增字段（与 `browser: PreviewPane` 并列，不挂在 `PreviewPane` 上——收藏夹跟文件
预览 tab 无关，只服务浏览器域）：

```rust
bookmarks: Vec<BookmarkInfo>,        // 本地缓存：全局 + 当前项目
browser_bookmarks_open: bool,        // 下拉面板开合
browser_star_menu_open: bool,        // 星标小菜单开合
```

刷新时机：`adopt_project`（项目打开/切换）时序里追加一次 `ListBookmarks{project_id}`，跟
`conversations`/`usage` 现有的刷新点挂在一起。未打开项目时（`project: None`）用
`ListBookmarks{project_id: None}`，缓存里只有全局收藏。

加入/移除走乐观本地更新：`Message::BrowserStarAdd(scope)`/`BrowserStarRemove(id)` 处理时先
本地改 `bookmarks`（星标立即变实心/空心，面板立即增删条目），同时向 dozerd 发对应请求；
失败复用 `browser_error` 展示文案，不做显式回滚——下次 `ListBookmarks` 刷新会用服务端真实
状态纠正，接受这个短暂不一致窗口（同 `todo.md` 写入冲突"静默放弃+下次覆盖"的降级哲学）。

### 4. UI

**地址栏星标**（`browser_pane` 函数内，"输入网址"按钮右侧）：新增
`icons::IconKind::Star` 图标按钮，`Message::BrowserStarClick`。当前激活 tab 若非
`TabKind::Web` 则该按钮禁用（文件/验收 tab 没有网址可收藏）。颜色判定：当前 URL 在
`ws.bookmarks` 里任一条 `url` 匹配 → GOLD 实心；否则 DIM 空心。

点击弹出小菜单（复用 `todo_dispatch_popup` 一类"`column![base, popup]` 内联挂靠"的写法）：
- 未收藏：两行按钮"加入全局收藏"/"加入本项目收藏"（`ws.project.is_none()` 时后一行禁用）。
- 已收藏：对应行文案变成"移出全局收藏"/"移出本项目收藏"（打勾态）。

**收藏夹面板**（tab 栏靠右，新增 `icons::IconKind::Bookmark` 图标按钮，
`Message::BrowserBookmarksToggle`）：下拉面板分两组渲染，"全局收藏"标题 + 该组条目，
"本项目收藏"标题 + 该组条目（无项目时不渲染这组，或渲染置灰的"未打开项目"提示——取前者，
更干净）。每条 = 标题文本（点击触发 `Message::BrowserOpenUrl(url)`，复用现有新开 tab
逻辑）+ 右侧 `×` 删除按钮（样式照抄 tab 关闭按钮，触发 `Message::BrowserStarRemove(id)`）。
两组都为空时显示"暂无收藏"占位文案（同现有"暂无网页——在地址栏输入网址"的空态风格）。

两个新图标手绘路径风格与 `icons.rs` 现有图标一致（矢量线条，非位图资源）。

## 错误处理

- dozerd 侧 `AddBookmark` 重复插入 → `INSERT OR IGNORE` 吞掉冲突，返回已存在的那条记录，不
  报错（幂等）。
- `RemoveBookmark` 传入不存在的 `id` → no-op，不报错（同 `bump_reload` 对未知 tab id 的处理
  哲学）。
- dozer-app 侧请求失败（daemon 未启动/UDS 断开）→ 复用 `browser_error` 字段展示错误文案，
  本地已经乐观更新的状态保留，不强制回滚。
- `ListBookmarks` 失败 → 缓存保持上一次成功的内容（或初始空），不清空、不阻断浏览器域其余
  功能。

## 测试策略

- `dozerd::bookmarks`：增删查基本路径；`scope=Global`/`Project` 各自的唯一索引生效（重复
  add 幂等，不报错、不重复插入）；`list(project_id)` 正确合并全局+项目、`list(None)` 只返回
  全局；`remove` 对不存在 id 是 no-op。
- 协议层：`AddBookmark`/`RemoveBookmark`/`ListBookmarks` 的 `encode_line`/序列化往返测试，
  照抄现有 `RecordAcceptance`/`OpenProject` 测试模式。
- `dozer-app`：
  - 星标态判定函数（给定当前 URL + `bookmarks` 缓存 → 是否命中/命中哪些 scope）的纯函数
    单测。
  - 乐观更新：`BrowserStarAdd`/`BrowserStarRemove` 处理后 `ws.bookmarks` 立即反映预期变化
    （不依赖真实网络往返，daemon 调用用现有测试里 mock/跳过网络层的模式）。
  - 面板开合、星标小菜单开合的状态机单测。
  - 面板分组渲染的边界情况：无项目时只有全局组、两组皆空时的占位态（走现有"构造
    `Workspace` + 断言渲染前置状态"的单测风格，不做真实 iced 渲染断言）。
- 真实视觉效果（图标手绘、下拉面板定位、GOLD/DIM 配色）留给人工验收（`cargo run -p
  dozer-app` 目测），同 Todo 面板等既有惯例，不做自动化 GUI 测试。

## 依赖变更

无新增依赖（`rusqlite` 已是 `dozerd` 现有依赖）。
