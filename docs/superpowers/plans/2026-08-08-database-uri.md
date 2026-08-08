# 数据库面板 · 数据源连接表 URI 支持 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 数据源表单新增可选"连接 URI"字段:用户整串粘贴
`postgres://...`/`mysql://...`,保存时以 URI 为准回填 host/port/database/
username,密码自动抽进 macOS Keychain,URI 脱敏落盘(`database.json` 无明文)。
不填 URI 走阶段 1 逐字段表单,零破坏。

**Architecture:** 在既有 `extensions::database.rs` 内追加,不改内核
(`workspace.rs` 的 `App::update`/`view`)。新增 `DataSource.uri:
Option<String>` 字段、`DataSourceDraft.uri`、`Message::DraftUriChanged`。
保存时 `parse_connection_uri` 抽取并脱敏;连接时 `build_sql_url` 改为
"URI 优先"并在最后一步 `inject_password_into_uri` 补回 Keychain 密码。

**Tech Stack:** Rust workspace;iced 0.14;`url` crate 已是 `sqlx-core`/`wry`
传递依赖(v2.5.8),显式 `url = "2"` 是标注直接依赖、零新增编译物;其余
(sqlx/mongodb/keyring)阶段 1 已引入。

## Global Constraints

- **密码绝不写入 `.dozer/database.json` 或日志**——只经 `keyring::Entry`
  存取(同阶段 1 约束)。URI 里的密码保存前必须抽出脱敏。
- **URI 只在保存时一次性抽取字段**;连接/测试/浏览共用 `build_sql_url`。
  `build_mongo_url` 与 SQLite 特殊路径**不碰**。
- **`database.json` 向后兼容**:新 `uri: Option<String>` 字段对旧文件解析为
  `None`,不加迁移。
- 每个任务结束 `cargo build && cargo test -p dozer-app database && cargo
  clippy --all-targets && cargo fmt` 干净通过(全 workspace)。
- 设计文档:`docs/superpowers/specs/2026-08-08-database-uri-design.md`(有
  疑问以它为准)。

> 说明:URI 功能已随 `feature/database-panel-phase2` 分支开发并单独 commit
> (`c2e9dff`)。本计划的任务项按**已落地代码 + 补充收尾验证**编排,重点是
> 核对每条 DoD 都达成,不再是"从零实现"。

---

### Task 1: 数据模型 + 依赖(已落地,核对)

**DoD:**

- [x] `Cargo.toml` 加 `url = "2"`,`Cargo.lock` 已含 `url 2.5.8`。
- [x] `DataSource.uri: Option<String>`(`database.rs:47-59`),注释写明密码
  不在此字段、不落 `database.json` 明文。

**操作:** 无代码改动。核对 `cargo tree -i url -p dozer-app` 确认 url 已被
sqlx-core/wry 拉起。

### Task 2: 解析与回注纯函数(已落地,核对 + 补测试)

`parse_connection_uri(uri) -> Option<ParsedUri>`(`database.rs:1099`)仅接受
postgres/postgresql/mysql → 抽 host/port/database/username/password → 脱敏
URI;失败返回 None。`inject_password_into_uri(uri, password)`
(`database.rs:1131`):URI 无密码且有 Keychain 密码才补回,自带密码保留。
`build_sql_url`(`database.rs:1041`)改为 URI 优先。

**DoD:**

- [x] 三函数实现与设计文档一致。
- [x] `url_tests` 模块(`database.rs:1769` 起)覆盖:
  extract+redact / mysql 无密码 / 拒绝非 DB scheme / sqlite 路径优先 /
  pg 有/无密码 / mongo 有密码 / inject 保自带密码 / build_sql_url URI 优先。

**操作:** 无代码改动,跑 `cargo test -p dozer-app database::url_tests` 全绿。

### Task 3: Draft 表单 + 保存决策(已落地,核对)

`DataSourceDraft.uri: String`(`database.rs:535`)、
`Message::DraftUriChanged(String)`(`database.rs:631`)、`DraftSave` 内部
(`database.rs:731-747`):URI 非空→parse 成功则回填字段+抽密码进 Keychain,
失败则忽略退化为字段式。URI 框仅 Postgres/MySQL 驱动渲染
(`database.rs:1354`),SQLite/MongoDB 不出现。

**DoD:**

- [x] URI 优先级正确:合法 URI 的 host/port/database/username 覆盖字段。
- [x] Keychain 抽取在保存时发生且只此一处;URI 脱敏落 `DataSource.uri`。
- [x] 驱动门:非 Postgres/MySQL 不渲染 URI 框。
- [x] 编辑已有源回显 URI(`database.rs:705`);密码框留空不改密码同阶段 1。

**操作:** 核对 `source_form` 的 URI 行渲染与 `DraftSave` 分支。

### Task 4: 单测补齐(DraftSave URI 路径)

`url_tests` 已覆盖纯函数。补 `DraftSave` 端到端路径用例如下。**注意关键
约束:** 本仓库测试不得写真实 Keychain(阶段 1 Global Constraints),故 URI 用例
统一用**无密码的 URI**(`uri_pw = None` → `DraftSave` 分支不触 Keychain
写入);"密码抽出→Keychain" 逻辑已由纯函数单测 `parse_uri_extracts_password_
and_redacts` 覆盖,不在 `DraftSave` 层重复。

**DoD:**

- [ ] `DraftSave` 合法(无密码)URI:粘 `postgresql://alice@db:5433/shop` →
      `DataSource.uri == "postgresql://alice@db:5433/shop"`(脱敏、无密码)、
      `host=="db"`/`port==5433`/`database=="shop"`/`username=="alice"` 由 URI
      回填;驱动为 Postgres。
- [ ] `DraftSave` 非法 URI:`host` 填合法字段但 `uri` 粘 `https://x` → 退化
      字段式,落盘 `DataSource.uri == None`,host 等取字段值,不 panic。
- [ ] 全部 database 测试绿(含既有 32 个 + 新增 2 个)。

### Task 5: 全量验证 + 人工验收

**DoD:**

- [ ] `cargo build`(workspace)干净。
- [ ] `cargo test -p dozer-app database` 全绿。
- [ ] `cargo clippy --all-targets` 无 `^error`,无 dead_code/unused warning。
- [ ] `cargo fmt --check` 干净。
- [ ] 人工验收:真拷云库 Postgres URI 粘贴保存 → 卡片 host 为 URI 回填、测试
      连接成功、`database.json` 无密码明文;MongoDB/SQLite 表单无 URI 框。
