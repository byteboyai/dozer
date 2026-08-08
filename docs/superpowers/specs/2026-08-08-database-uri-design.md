# 数据库面板 · 数据源连接表 URI 支持

**状态:已实现(commit `c2e9dff`,文档后补)**

## 背景

阶段 1(驱动管理 + 数据源 CRUD + 连接测试,commit `d8fcecb`,见
`docs/superpowers/specs/2026-08-08-database-panel-phase1-design.md`)把数据源
的录入做成"逐字段表单":host / port / database / username / password 分开填。
这在早期够用,但真实项目里连接串往往以一段 URI 交付(云数据库的
连接面板、IDE / CLI 导出、同事分享的同步串),逐字段搬抄既慢又容易抄错
(密码多一个 `@`、schema 没填、port 拼错)。

本增强:数据源表单新增一个**可选的"连接 URI"字段**。用户把整串
`postgres://...`/`mysql://...` 粘进去,保存时**以 URI 为准**回填 host /
port / database / username,并**自动把密码抽进 Keychain**(URI 本身脱敏,
`database.json` 不留明文)。不填 URI 则一切照旧走逐字段表单——对既有
工作流零破坏。

调研核实(写代码前已验证):

- sqlx 的连接串构造在 `build_sql_url`(阶段 1 已有);URI 优先 = 当
  `DataSource.uri` 非空且以 `://` 开头时,**原样**当连接串用,不再拼字段。
  SQLite 特殊路径不变(`sqlite://<数据库路径>`,URI 字段对 SQLite 无意义)。
- `url::Url` 已是 `sqlx-core` / `wry` 的传递依赖(v2.5.8,见 `cargo tree
  -i url`)。显式把 `url = "2"` 加进 `dozer-app/Cargo.toml` 只是**标注**
  直接依赖,**不新增任何编译物**——沿用"零新增依赖"的仓库约束。
- sqlx-any 只接原生连接串;URI 里的密码若抽走后补回必须保证 URL 子串
  顺序正确(`set_password` 由 `url` 库保证,不手拼)。

## 目标 / 非目标

**目标**:

1. 表单加"连接 URI"输入框(仅 Postgres/PostgreSQL/MySQL 驱动出现;
   SQLite 是文件路径、MongoDB 走 `mongodb://` 串,均不适用)。
2. 保存时 URI 优先:合法 URI(可解析、scheme 是 postgres/postgresql/mysql)
   用它的 host/port/database/username 回填字段,密码抽到 Keychain,URI
   脱敏落盘。**不合法的 URI 静默忽略**,退化为逐字段表单(不报错打断,
   用户可自行改字段)。
3. `DataSource` 新增 `uri: Option<String>` 字段——会话内 + `database.json`
   均持久化此字段(raw、无密码)。
4. 连接/测试/浏览共用同一连接串构造:URI 存在则原样用它,连接时把
   Keychain 密码补回(URI 自身已带密码则保留原样)。

**非目标**(明确不做,避免蔓延):

- 不支持 SQLite/MongoDB 的 URI 字段(语义不符,界面不出现)。
- 不做"URI → 字段"的反向冲刷回填(fill-back):URI 只在**保存时**一次性
  抽取字段;保存后表单不因 URI 重排,编辑已有源也不自动从 URI 重估。
- 不做连接串健康度预校验(能否连上仍是保存/测试时才知道,同阶段 1)。
- 不做 MySQL `mysql://` 带 `?ssl-mode` 之类的 query 参数解析——URI 原样
  透传给 sqlx,MQTT 参数由 sqlx 消费,客户端不解析。
- 不新增任何依赖(见"依赖变更"节)。

## 架构与数据流

### 1. 数据模型

`DataSource` 加 `pub uri: Option<String>`(见 `database.rs:47-59`)。注释明确
密码不在此字段、也不在 `database.json` 落地明文。

### 2. 表单与评审 draft

`DataSourceDraft` 加 `pub uri: String`、`Message::DraftUriChanged(String)`,
`DraftSave` 内部按以下顺序决策(见 `database.rs:731-747`):

```text
如果 draft.uri 非空:
    parse_connection_uri(draft.uri)
        成功  -> DataSource.uri   = 脱敏后 URI
                host/port/database/username = 从 URI 抽取
                password = 从 URI 抽出 -> Keychain
        解析失败/scheme 不支持 -> 忽略 URI,退化为字段式(build_sql_url 回填)
否则   -> 字段式,同阶段 1
```

关键:URI 优先级高于字段。即使用户把 host/port/username 又填得与 URI
不一致,保存结果永远以 URI 为准——因为连接时 `build_sql_url` 只在
`uri` 缺失时才拼字段。

### 3. 解析与回注(纯函数,单测友好)

`parse_connection_uri(uri) -> Option<ParsedUri>`(`database.rs:1099`):
仅 `postgres`/`postgresql`/`mysql` scheme;返回脱敏后的 uri、
host(必填,缺失即 None)、port、database(取 path 首段)、username、password;
解析完成用 `Url::set_password(None)` 脱敏回写。

`inject_password_into_uri(uri, password) -> String`(`database.rs:1131`):
连接/测试时调,URI 无密码且给了 Keychain 密码才 `set_password` 补回;
URI 自带密码则原样保留。

`build_sql_url`(`database.rs:1041`)改为:
`uri 存在且含 "://"` → `inject_password_into_uri(uri, password)`;
否则字段式拼接(阶段 1 原逻辑)。SQLite 分支在最前,不经 URI 字段。

`build_mongo_url` 不动。

### 4. `database.json` 兼容

`uri` 是 `Option<String>`,旧文件(无此字段)反序列化自然为 `None`,阶段 1
存量 `database.json` 不清零、不加迁移。含密码的旧 URI?(不安全,见
"安全"节)。

## 安全

- 密码生命周期:录入 → `parse_connection_uri` 抽出 → 写 Keychain → 提交
  `DataSource.uri` 已脱敏。URI 永远不以含密码形态落盘。
- 编辑已有源时 URI 原样回显(`database.rs:705`);密码框留空 = 不改密码
  (同阶段 1 惯例),`build_sql_url` 对 URI 场景用 Keychain 补回。
- 若用户粘贴的 URI **本身带密码**,解析时先抽出、落 Keychain、落盘脱敏。
  `inject_password_into_uri` 连接时见 URI 无密码才补回 → 不会出现"明文 URI
  落盘又补一份密码"。

## UI 与视觉

- 表单在 password 行下方加"连接 URI(可选)"输入行,`DraftUriChanged` 驱动。
  仅当 `draft.driver` ∈ {Postgres, MySQL}(MongoDB/SQLite 不渲染,减少噪音)。
- 表单下方小字提示(可含 style,若已存在):"填了连接 URI 将优先使用,密码
  自动保存到系统钥匙串"。沿用阶段 1 caption 风格与 14 色主题令牌,不新增色。
- `source_card` 摘要区:URI 源额外标一行小字"URI"或直接高亮 host/port 已
  由 URI 回填(阶段 1 卡片本就显示 host/port,URI 优先后展示值即 URI 来的,
  无需额外代码;可选:加了 `uri` 时某处标注,见"已实现"决定取舍)。

## 错误处理

- URI 文本不合法 → 保存时静默忽略(退化为字段式),不弹错、不打断——用户
  可回头改字段,或改对 URI 再存。这是刻意的低摩擦,与"URI 是可选便利"
  的定位一致。
- Keychain 读不到密码 → 视为无密码,认证失败由驱动报具体错(同阶段 1)。
- URI scheme 有效但 host 缺失 → `parse_connection_uri` 返回 None → 同上
  退化字段式(host 缺失不可能靠字段拼出去,等用户补)。

## 测试策略

- 纯函数单测(`url_tests`,`database.rs:1769` 起):
  - `parse_uri_extracts_password_and_redacts`——抽出密码、URI 脱敏。
  - `parse_uri_mysql_without_password`——mysql scheme、无密码。
  - `parse_uri_rejects_non_db_schemes`——mongodb/http 等拒绝。
  - `sqlite_url_uses_database_as_path_ignores_host`——SQLite 路径优先。
  - `postgres_url_with_password` / `postgres_url_without_password`。
  - `mongo_url_with_password`。
  - `inject_password_keeps_existing_uri_password`——自带密码不覆盖。
  - `uri_takes_precedence_in_build_sql_url`——URI 优先,字段被忽略。
- 既有 database 测试沿用;`DraftSave` URI 路径在 `mod tests` 加端到端用例
  (见计划 Task 4)。因本仓库测试**不得写真实 Keychain**,用例统一用无密码
  URI("密码抽进 Keychain" 由纯函数 `parse_uri_extracts_password_and_redacts`
  覆盖,不触真实钥匙串)。
- 人工验收:真拷一条云库 Postgres URI 粘贴保存 → 卡片显示 URI 回填的
  host、测试连接成功;`database.json` 里 `uri` 无密码明文;MongoDB/SQLite
  表单无 URI 框。

## 依赖变更

`dozer-app/Cargo.toml` 加 `url = "2"`(显式直接依赖)。`url` 已被
`sqlx-core`/`wry` 锁定加载(v2.5.8),**不发新编译物、不新增任何依赖**。
其余(sqlx/mongodb/keyring)阶段 1 已引入。

## 排期

- 基线:`feature/database-panel-phase2` 分支(URI 工作已随其开发,单独 commit
  `c2e9dff`)。
- 实现计划:`docs/superpowers/plans/2026-08-08-database-uri.md`。
