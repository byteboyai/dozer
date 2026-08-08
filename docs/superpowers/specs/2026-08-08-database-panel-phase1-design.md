# 数据库面板 · 阶段 1:驱动管理 + 数据源 CRUD + 连接测试

**状态:已批准(brainstorming 会话,2026-08-08)**

## 背景

用户提出要给 Dozer 加一个数据库面板:管理数据源、浏览数据库表/集合结构、浏览表
数据、执行 SQL 脚本。brainstorming 过程中调研了 Rust 生态里可用的库(见对话
记录,未另存独立文档):

- 连接层用 [SQLx](https://github.com/launchbadge/sqlx)——`AnyPool`/
  `AnyConnection` 支持按连接串 scheme 在运行时选驱动(Postgres/MySQL/
  MariaDB/SQLite 一套 API),不需要为每种关系型数据库分别写连接代码。
- MongoDB 是文档数据库,API 与 SQLx 完全不同,用官方 [mongodb](https://
  crates.io/crates/mongodb) crate,数据源抽象层要做一次 SQL/NoSQL 分叉。
- schema introspection 没有真正通用的现成 crate,需要按数据库类型各自手写
  查系统目录的 SQL(Postgres `information_schema`/`pg_catalog`,MySQL
  `information_schema`,SQLite `sqlite_master`)——这是后续阶段的工作,这次
  不做。
- 数据网格渲染:核对过本仓库 `Cargo.lock` 锁定的 `iced_widget` 版本是
  **0.14.2**,已经内置原生 `iced_widget::table` 模块,不需要额外引入
  `iced_table` 这个第三方 crate——这是后续"数据浏览"阶段要用到的,这次
  不用。

这次范围经用户确认按**功能面**(而非按"每种驱动一个阶段")切成 5 块:
① 驱动管理 + 数据源 CRUD + 连接测试(本设计文档范围)、② schema 树浏览、
③ 数据表格浏览、④ SQL 脚本执行、⑤ MongoDB 集合浏览。①-④ 是关系型数据库
(Postgres/MySQL/SQLite)的完整闭环,⑤ 单独因为 MongoDB 没有"执行 SQL 脚本"
这个概念(已确认这次只做集合浏览,不做查询/脚本入口)。本文档只覆盖①,
后续 4 块各自单独走一轮 brainstorming 定设计,不在这里预先写死。

**排期**:与 `App`/`Workspace` 文件拆分(`docs/superpowers/specs/2026-08-08-
app-workspace-file-split-design.md`,进行中,另一个 agent 在
`feature/app-workspace-split` 分支实现,本 session 待通知审阅)相互独立,
不冲突——这次新建的是 `extensions::database.rs`,不碰 `workspace.rs`/`app.rs`
内部结构;唯一交叉点是 `workspace.rs`(或拆分完成后的 `app.rs`,视这个功能
排期在拆分之前还是之后开工)里 `LeftView` 加一个新变体、`left_icon_rail`/
`left_panel_area` 加一个分支——改动量很小,真发生分支排期冲突时按现有惯例
(先落地哪个就先合并哪个,后合并的一方 rebase)处理,不预先规划复杂的合并
顺序。

## 目标 / 非目标

**目标**:

1. 新建 `crates/dozer-app/src/extensions/database.rs`,结构照搬
   `extensions::todo` 的双状态模式(`AppState` 挂 App 级、`WorkspaceState`
   挂每个 `Workspace`):
   - `AppState`:`enabled_drivers: HashSet<DriverKind>`(默认全部启用),
     持久化到 `dozer_core::paths::config_dir().join("database_drivers.
     json")`(与 `layout.json`/`todo_meta.json` 同一目录、同一
     `serde_json` 读写模式,读失败/文件不存在按默认值处理,不 panic)。
   - `WorkspaceState`:`sources: Vec<DataSource>`(当前项目配置的数据源列表)
     + 新建/编辑表单草稿态 + 每条数据源的连接测试状态(`HashMap<String,
     TestStatus>`,`TestStatus = Idle | Testing | Ok | Err(String)`),
     `sources` 持久化到 `.dozer/database.json`(与 `.dozer/goal.md`/
     `.dozer/todo.md` 同一目录,但用 JSON——这是给程序读写的结构化配置,不是
     给人手改的文档,不套用 goal.md/todo.md 的 Markdown 自定义格式)。
2. 数据模型:

   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
   pub enum DriverKind {
       Postgres,
       MySQL,
       Sqlite,
       MongoDB,
   }

   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct DataSource {
       pub id: String,       // uuid v4,新建时生成
       pub name: String,     // 用户起的名字,面板卡片标题
       pub driver: DriverKind,
       pub host: Option<String>,     // SQLite 不需要
       pub port: Option<u16>,        // SQLite 不需要
       pub database: Option<String>, // SQLite 这里存文件路径
       pub username: Option<String>, // SQLite/部分 MongoDB 场景可能没有
   }
   ```

   **密码不进 `DataSource`/`.dozer/database.json`**——单独走 macOS Keychain
   (`keyring` crate),key 用 `service = "dozer", account = format!("{project_
   id}:{data_source_id}")` 拼(同一把钥匙串条目按项目+数据源双重区分,避免
   两个项目各自起同名数据源时互相覆盖密码)。SQLite 驱动因为不需要密码
   (本地文件),不触碰 Keychain。
3. 面板接线:新增 `LeftView::Database` + `RailButton::LeftDatabase`(图标
   写计划时从 Lucide 选,如 `database`)。驱动启用/禁用**不建独立的 App
   设置页**——现状顶栏"设置"图标只有视觉、完全没接线,借这个功能顺带建
   一整套设置页基础设施是范围外扩张;做成面板顶部一个"管理驱动"按钮,点开
   一个勾选列表小弹层(4 个 `DriverKind` 逐个勾/不勾),关闭即保存,足够。
4. `view` 结构:面板顶部"管理驱动"入口(见上)+ 当前项目数据源卡片列表
   (每张卡:名字 + 驱动类型图标/文案 + host:port/database 摘要 + "测试
   连接"按钮 + 编辑/删除按钮)+ "＋新增数据源"按钮。新增/编辑走同一个表单
   弹层:驱动下拉(**只列已启用的驱动**;如果表单打开时某个正在编辑的
   已存在数据源的驱动被禁用了,下拉里额外把它加回去并标注"已禁用",不强迫
   用户先改驱动才能保存别的字段)+ host/port/database/username 文本框(
   按所选驱动动态显示/隐藏——SQLite 只显示"文件路径"一个字段,MongoDB 不
   强制要求 username)+ 密码框(`text_input` 的 `secure()` 模式,同现有仓库
   密码类输入的既有写法——若现状没有先例,写计划时核对 iced 0.14 的
   `text_input::Style`/`secure` API)。
5. 连接测试(异步):点"测试连接"→ `TestStatus::Testing`(卡片显示 spinner
   或"测试中…"文案)→ 按 `driver` 分派:
   - `Postgres`/`MySQL`/`Sqlite`:拼连接串(host/port/database/username +
     从 Keychain 取的密码),`sqlx::AnyPool::connect` + 一次 `SELECT 1`
     (SQLite 用 `SELECT 1` 一样有效)验证真的能查询,不只是能建连接。
   - `MongoDB`:`mongodb::Client::with_uri_str` + `.list_database_names()`
     或等价的 ping 操作。
   - 整个测试套 5 秒超时(`tokio::time::timeout`)——网络不可达时不能让
     UI 一直转圈。
   - 结果经 `emit`/`EventLoopProxy` 回传 `Message::TestConnectionResult
     (data_source_id, Result<(), String>)`,落地成 `TestStatus::Ok` /
     `TestStatus::Err(错误文案)`,卡片上直接显示(绿勾/红叉+简短错误)。
6. `update` 签名(照搬 `browser`/`git_log` 那套异步 extension 的
   `handle`+`emit` 风格,不是 `todo` 那套同步风格——因为这个模块的核心操作
   `TestConnection` 本质是网络 IO):

   ```rust
   pub fn update(
       ws_state: &mut WorkspaceState,
       app_state: &mut AppState,
       msg: Message,
       project_id: i64,
       repo_path: &Path,
       handle: &tokio::runtime::Handle,
       emit: impl Fn(Message) + Send + 'static,
   )
   ```

**非目标**(留给后续 4 个阶段各自单独设计,这次不做):

- 不做 schema 树(表/集合结构浏览)。
- 不做数据表格浏览(`iced_widget::table` 集成留后续)。
- 不做 SQL 脚本执行、不做破坏性语句二次确认弹窗(已确认要做,但属于阶段 4
  范围,这次的"测试连接"只跑一条内部固定的 `SELECT 1`,不暴露任何用户可
  编辑的 SQL 输入框)。
- 不做 MongoDB 集合浏览。
- 不建通用 App 设置页——驱动管理就是面板内一个小弹层,不是独立路由/页面。
- 不做自定义/外接驱动插件机制——`DriverKind` 是穷举枚举,新增驱动类型是
  以后的事,这次固定四个。
- 连接串本身(不含密码的部分)不加密——host/port/database/username 是明文
  存 `.dozer/database.json`,理由同 `layout.json`/`todo_meta.json` 现状:
  这些是本地开发机上的配置,不是需要保密的凭据本身,只有密码单独保护。

## 架构与数据流

### 1. 文件与模块

- 新建 `crates/dozer-app/src/extensions/database.rs`,`extensions.rs` 加
  `pub mod database;`。
- `AppState`/`WorkspaceState`/`Message`/`update`/`view` 全部在这一个文件里
  (照搬 `todo.rs` 的组织方式,不像 `files.rs` 那样再拆出编辑态子模块——
  阶段 1 状态量不大,没必要提前拆)。

### 2. 持久化细节

```rust
fn drivers_path() -> PathBuf {
    dozer_core::paths::config_dir().join("database_drivers.json")
}
fn sources_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("database.json")
}
```

`AppState::load()`/内部 `save()` 与 `WorkspaceState` 的 sources 读写,套用
`todo.rs` 里 `meta_load`/`meta_save`/`load_from`/`save_to` 的既有模式:读
失败或文件不存在返回默认值(`AppState` 默认全部驱动启用;`WorkspaceState`
默认空列表),不 panic;写失败记 `tracing::warn!`,不中断交互。

密码读写走 `keyring::Entry::new("dozer", &format!("{project_id}:{source_
id}"))`,`get_password()`/`set_password()`/`delete_password()`。删除数据源
时必须连带删 Keychain 里的密码条目,不留孤儿凭据。

### 3. 连接测试的驱动分派

```rust
async fn test_connection(source: DataSource, password: Option<String>) -> Result<(), String> {
    match source.driver {
        DriverKind::Postgres | DriverKind::MySQL | DriverKind::Sqlite => {
            let url = build_sql_url(&source, password.as_deref());
            let pool = sqlx::AnyPool::connect(&url).await.map_err(|e| e.to_string())?;
            sqlx::query("SELECT 1").execute(&pool).await.map_err(|e| e.to_string())?;
            Ok(())
        }
        DriverKind::MongoDB => {
            let url = build_mongo_url(&source, password.as_deref());
            let client = mongodb::Client::with_uri_str(&url).await.map_err(|e| e.to_string())?;
            client.list_database_names().await.map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}
```

`build_sql_url`/`build_mongo_url` 是两个纯函数(输入 `DataSource` + 密码,
输出连接串),写计划时按各驱动的 URL scheme 精确定义(`postgres://`/
`mysql://`/`sqlite://`/`mongodb://`)。`sqlx::Any` 需要先
`sqlx::any::install_default_drivers()`(程序启动时调一次,类似字体注册,
放 `main.rs` 初始化段)。

`Message::TestConnection(source_id)` 处理器里,`handle.spawn(async move {
let result = tokio::time::timeout(Duration::from_secs(5), test_connection(
source, password)).await; let result = match result { Ok(r) => r, Err(_) =>
Err("连接超时(5秒)".into()) }; emit(Message::TestConnectionResult(source_
id, result)); })`。

## 错误处理

- `.dozer/database.json`/`database_drivers.json` 读失败(格式损坏/权限
  问题)→ 按默认值兜底,不崩,面板正常显示(空列表/全部驱动启用)。
- Keychain 读写失败(用户拒绝授权/系统限制)→ 密码字段视为空,连接测试会
  自然失败并报出"未找到密码"类错误,不 panic、不静默吞掉。
- 连接测试失败(网络不可达/认证失败/超时)→ 卡片上展示具体错误文案(来自
  驱动库的错误消息,必要时截断长度),不重试、不自动退避——用户手动再点
  一次"测试连接"。
- 表单校验:必填字段(name/driver;host/port/database 按驱动类型各自的
  必填规则,SQLite 只要求文件路径)留空时"保存"按钮禁用或提交时给行内
  错误提示,写计划时定具体交互,不在设计阶段精确到每个字段的校验规则。

## 测试策略

- `build_sql_url`/`build_mongo_url` 纯函数单测:各驱动类型给定字段生成的
  连接串符合预期格式(不含真实网络连接,不需要 mock)。
- `AppState`/`WorkspaceState` 的加载/保存往返测试:同 `todo.rs` 现有
  `save_then_load_round_trips_and_fields_are_independent` 一类的模式,验证
  序列化/反序列化字段独立、缺文件返回默认值。
- `test_connection` 本身依赖真实网络/真实数据库,不适合单测覆盖——写计划
  时看是否值得起一个本地临时 SQLite 文件做"连接一个真实存在的 SQLite 库"
  的集成测试(SQLite 不需要外部服务,风险低);Postgres/MySQL/MongoDB 需要
  真实服务,不在单测范围内,人工验收覆盖。
- 人工验收:驱动管理弹层勾选/取消能正确影响"新增数据源"表单的驱动下拉;
  新增/编辑/删除数据源后重启 app,`.dozer/database.json` 里的数据还在,
  密码从 Keychain 正确取回;对一个真实可达的 SQLite 文件/Postgres 实例点
  "测试连接"能看到成功;对一个不存在的 host 点"测试连接"5 秒内看到超时
  错误,不是无限转圈。

## 依赖变更

新增:
- `sqlx`(features:`postgres`、`mysql`、`sqlite`、`any`、`runtime-tokio`、
  `tls-rustls` 或 `tls-native-tls`——写计划时按仓库现有 TLS 依赖选型对齐,
  避免同时拉两套 TLS 实现)。
- `mongodb`(默认 features,`async-std`/`tokio` 二选一运行时,选
  `tokio`——与仓库现有 `tokio::runtime::Handle` 一致)。
- `keyring`(macOS 后端走 Security.framework,不需要额外系统依赖;这个
  crate 本身跨平台但这次只用 macOS 路径,符合"mac 先发但架构留门"的裁决
  ——不排除以后其它平台走各自的凭据后端)。
