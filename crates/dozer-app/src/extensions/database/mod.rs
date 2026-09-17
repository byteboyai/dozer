//! 数据库面板 · 阶段 1:驱动管理 + 每项目数据源 CRUD + 连接测试。结构照搬
//! `extensions::todo` 的双状态模式——`AppState` 挂 `App`(全局:哪些驱动
//! 启用),`WorkspaceState` 挂每个 `Workspace`(当前项目的数据源列表)。
//! 密码不进这个文件的任何持久化结构,单独走 macOS Keychain(见
//! `keyring_key`)。schema 树/数据浏览/SQL 执行/MongoDB 集合浏览留后续
//! 阶段,见
//! `docs/superpowers/specs/2026-08-08-database-panel-phase1-design.md`。

mod load;
mod state;
mod update;
mod view;

pub(crate) use load::*;
pub(crate) use state::*;
pub(crate) use update::*;
pub(crate) use view::*;

#[cfg(test)]
mod url_tests {
    use super::*;

    fn base(driver: DriverKind) -> DataSource {
        DataSource {
            id: "x".into(),
            name: "n".into(),
            driver,
            host: Some("db.internal".into()),
            port: Some(5432),
            database: Some("mydb".into()),
            username: Some("alice".into()),
            uri: None,
        }
    }

    #[test]
    fn postgres_url_with_password() {
        let s = base(DriverKind::Postgres);
        assert_eq!(
            build_sql_url(&s, Some("secret")),
            "postgres://alice:secret@db.internal:5432/mydb"
        );
    }

    #[test]
    fn postgres_url_without_password() {
        let s = base(DriverKind::Postgres);
        assert_eq!(
            build_sql_url(&s, None),
            "postgres://alice@db.internal:5432/mydb"
        );
    }

    #[test]
    fn sqlite_url_uses_database_as_path_ignores_host() {
        let mut s = base(DriverKind::Sqlite);
        s.database = Some("/tmp/my.sqlite".into());
        assert_eq!(build_sql_url(&s, None), "sqlite:///tmp/my.sqlite");
    }

    #[test]
    fn mongo_url_with_password() {
        let s = base(DriverKind::MongoDB);
        assert_eq!(
            build_mongo_url(&s, Some("secret")),
            "mongodb://alice:secret@db.internal:5432"
        );
    }

    #[test]
    fn parse_uri_extracts_password_and_redacts() {
        let p =
            parse_connection_uri("postgresql://alice:SuperSecret@db.internal:5433/shop").unwrap();
        assert_eq!(p.host.as_deref(), Some("db.internal"));
        assert_eq!(p.port, Some(5433));
        assert_eq!(p.database.as_deref(), Some("shop"));
        assert_eq!(p.username.as_deref(), Some("alice"));
        assert_eq!(p.password.as_deref(), Some("SuperSecret"));
        // 脱敏:密码已抽走,不留在 uri 里。
        assert!(!p.uri.contains("SuperSecret"));
        assert!(p.uri.contains("@db.internal:5433/shop"));
    }

    #[test]
    fn parse_uri_mysql_without_password() {
        let p = parse_connection_uri("mysql://root@db/shop").unwrap();
        assert_eq!(p.host.as_deref(), Some("db"));
        assert_eq!(p.database.as_deref(), Some("shop"));
        assert_eq!(p.username.as_deref(), Some("root"));
        assert!(p.password.is_none());
        assert_eq!(p.uri, "mysql://root@db/shop");
    }

    #[test]
    fn parse_uri_rejects_non_db_schemes() {
        assert!(parse_connection_uri("https://db/shop").is_none());
        assert!(parse_connection_uri("not-a-uri").is_none());
    }

    #[test]
    fn uri_takes_precedence_in_build_sql_url() {
        let mut s = base(DriverKind::Postgres);
        s.uri = Some("postgresql://alice@db:5433/shop".into());
        // 无 keychain 密码:直接抄 URI。
        assert_eq!(build_sql_url(&s, None), "postgresql://alice@db:5433/shop");
        // 有 keychain 密码且 URI 无密码:补回。
        assert_eq!(
            build_sql_url(&s, Some("secret")),
            "postgresql://alice:secret@db:5433/shop"
        );
    }

    #[test]
    fn inject_password_keeps_existing_uri_password() {
        assert_eq!(
            inject_password_into_uri("postgresql://alice:pw@db/shop", Some("other")),
            "postgresql://alice:pw@db/shop"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashSet;

    fn ds(id: &str, driver: DriverKind) -> DataSource {
        DataSource {
            id: id.into(),
            name: format!("test-{id}"),
            driver,
            host: Some("localhost".into()),
            port: Some(5432),
            database: Some("mydb".into()),
            username: Some("user".into()),
            uri: None,
        }
    }

    #[test]
    fn app_state_defaults_to_all_drivers_enabled() {
        let state = AppState::default();
        for d in DriverKind::ALL {
            assert!(state.is_enabled(d));
        }
    }

    #[test]
    fn app_state_load_missing_file_returns_default() {
        // config_dir() 指向真实用户目录,这里只验证"文件不存在"分支不 panic
        // 且落到全启用默认值——不清真实用户配置,不需要临时目录隔离(同
        // `icon_size.rs` 现有测试对 `config_dir()` 的处理口径:该文件本就
        // 可能不存在,读不到就是默认值,这条本身不依赖文件真的不存在)。
        let path = drivers_path();
        if !path.exists() {
            let state = AppState::load();
            assert!(state.is_enabled(DriverKind::Postgres));
        }
    }

    #[test]
    fn toggle_flips_membership() {
        let mut state = AppState::default();
        state.toggle(DriverKind::MongoDB);
        assert!(!state.is_enabled(DriverKind::MongoDB));
        state.toggle(DriverKind::MongoDB);
        assert!(state.is_enabled(DriverKind::MongoDB));
    }

    #[test]
    fn reload_from_disk_missing_file_yields_empty_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.sources().is_empty());
    }

    #[test]
    fn save_then_reload_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let sources = vec![ds("a", DriverKind::Postgres), ds("b", DriverKind::Sqlite)];
        save_sources(dir.path(), &sources).unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert_eq!(ws_state.sources(), sources.as_slice());
    }

    #[test]
    fn reload_from_disk_corrupt_file_yields_empty_sources() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        std::fs::write(sources_path(dir.path()), "not json").unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.sources().is_empty());
    }

    fn tree_table(schema: Option<&str>, name: &str, is_view: bool) -> TableRef {
        TableRef {
            schema: schema.map(|s| s.to_string()),
            name: name.into(),
            is_view,
        }
    }

    #[test]
    fn tree_rows_flat_for_sqlite() {
        let mut st = SchemaState {
            tables: vec![
                tree_table(None, "users", false),
                tree_table(None, "orders", true),
            ],
            ..Default::default()
        };
        st.expanded_tables.insert((None, "users".into()));
        st.columns.insert(
            (None, "users".into()),
            ColumnLoad::Loaded(vec![ColumnInfo {
                name: "id".into(),
                type_name: "INTEGER".into(),
                nullable: false,
            }]),
        );
        let rows = tree_rows(&st, DriverKind::Sqlite);
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0].kind, SchemaRowKind::Table(t) if t.name == "users"));
        assert_eq!(rows[0].depth, 0);
        assert!(rows[0].expanded);
        assert!(matches!(rows[1].kind, SchemaRowKind::Column(c) if c.name == "id" && !c.nullable));
        assert_eq!(rows[1].depth, 1);
        assert!(matches!(rows[2].kind, SchemaRowKind::Table(t) if t.name == "orders" && t.is_view));
    }

    #[test]
    fn tree_rows_postgres_groups_by_schema_and_folds() {
        let mut st = SchemaState {
            tables: vec![
                tree_table(Some("public"), "users", false),
                tree_table(Some("audit"), "events", false),
            ],
            ..Default::default()
        };
        // 未展开:只有两个 schema 行(BTreeMap 序 audit < public)
        let rows = tree_rows(&st, DriverKind::Postgres);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0].kind, SchemaRowKind::Schema("audit")));
        assert!(matches!(rows[1].kind, SchemaRowKind::Schema("public")));
        assert!(!rows[1].expanded);

        // 展开 public:表行挂在 depth 1
        st.expanded_schemas.insert("public".into());
        let rows = tree_rows(&st, DriverKind::Postgres);
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[2].kind, SchemaRowKind::Table(t) if t.name == "users"));
        assert_eq!(rows[2].depth, 1);
    }

    #[test]
    fn tree_rows_emits_loading_and_failed_placeholders() {
        let mut st = SchemaState {
            tables: vec![tree_table(None, "a", false), tree_table(None, "b", false)],
            ..Default::default()
        };
        st.expanded_tables.insert((None, "a".into()));
        st.expanded_tables.insert((None, "b".into()));
        st.columns.insert((None, "a".into()), ColumnLoad::Loading);
        st.columns
            .insert((None, "b".into()), ColumnLoad::Failed("nope".into()));
        let rows = tree_rows(&st, DriverKind::MySQL);
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, SchemaRowKind::ColumnsLoading) && r.depth == 1)
        );
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, SchemaRowKind::ColumnsFailed(e) if e == "nope"))
        );
    }

    fn update_with(ws: &mut WorkspaceState, msg: Message) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut app_state = AppState::default();
        let dir = tempfile::tempdir().unwrap();
        rt.block_on(async {
            update(ws, &mut app_state, msg, 42, dir.path(), rt.handle(), |_| ());
        });
    }

    /// Postgres 源 + 不可达端口:BrowseSchema/ToggleTable 触发的后台任务
    /// 要么不执行(单测 runtime),要么秒拒连,且不会有 SQLite 建文件副作用。
    fn pg_source(id: &str) -> DataSource {
        DataSource {
            id: id.into(),
            name: format!("test-{id}"),
            driver: DriverKind::Postgres,
            host: Some("127.0.0.1".into()),
            port: Some(1),
            database: Some("mydb".into()),
            username: Some("user".into()),
            uri: None,
        }
    }

    fn seeded_ws(source: DataSource) -> WorkspaceState {
        let mut ws = WorkspaceState {
            expanded_sources: HashSet::from([source.id.clone()]),
            ..Default::default()
        };
        ws.sources.push(source.clone());
        ws.schemas.insert(source.id.clone(), SchemaState::default());
        ws
    }

    #[test]
    fn browse_schema_first_time_starts_loading() {
        let src = pg_source("s1");
        let mut ws = WorkspaceState::default();
        ws.sources.push(src.clone());
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into()));
        assert!(ws.is_expanded("s1"));
        let st = ws.schema_state("s1").unwrap_or_else(|| panic!());
        assert!(st.loading_tables());
        assert!(st.tables().is_empty());
        assert!(st.tables_error().is_none());
    }

    #[test]
    fn browse_schema_second_time_uses_cache() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        // 模拟已加载完成:收起再展开
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into()));
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into()));
        let st = ws.schema_state("s1").unwrap();
        assert!(!st.loading_tables()); // 有缓存,不重拉
        assert_eq!(st.tables().len(), 1);
    }

    #[test]
    fn schema_back_clears_browsing_keeps_cache() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into()));
        assert!(!ws.is_expanded("s1"));
        assert_eq!(ws.schema_state("s1").unwrap().tables().len(), 1);
    }

    #[test]
    fn schema_refresh_requires_browsing_match() {
        let mut ws = seeded_ws(pg_source("s1"));
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into())); // 收起
        update_with(&mut ws, Message::SchemaRefresh("s1".into()));
        assert!(!ws.schema_state("s1").unwrap().loading_tables()); // 旧菜单残留防线
    }

    #[test]
    fn toggle_schema_flips_expansion() {
        let mut ws = seeded_ws(pg_source("s1"));
        let st = ws.schemas.get_mut("s1").unwrap();
        st.tables = vec![tree_table(Some("public"), "users", false)];
        update_with(&mut ws, Message::ToggleSchema("s1".into(), "public".into()));
        assert!(
            tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(_)))
        );
        update_with(&mut ws, Message::ToggleSchema("s1".into(), "public".into()));
        assert!(
            !tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(_)))
        );
    }

    #[test]
    fn toggle_table_expands_to_loading_then_loaded() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        // 展开 schema 节点,表行/列行才会进 tree_rows(Postgres 多一层)
        update_with(&mut ws, Message::ToggleSchema("s1".into(), "public".into()));
        // 首次展开 → Loading(spawn 的任务不会在本测试 runtime 里跑完)
        update_with(
            &mut ws,
            Message::ToggleTable {
                source_id: "s1".into(),
                schema: Some("public".into()),
                table: "users".into(),
            },
        );
        let key = (Some("public".to_string()), "users".to_string());
        assert!(matches!(
            ws.schema_state("s1").unwrap().column_load(&key),
            Some(ColumnLoad::Loading)
        ));
        // 结果落地 → Loaded,树里出现列行
        update_with(
            &mut ws,
            Message::ColumnsLoaded {
                project_id: 42,
                source_id: "s1".into(),
                schema: Some("public".into()),
                table: "users".into(),
                result: Ok(vec![ColumnInfo {
                    name: "id".into(),
                    type_name: "integer".into(),
                    nullable: false,
                }]),
            },
        );
        assert!(
            tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Column(c) if c.name == "id"))
        );
        // 过期结果(已不是 Loading)→ 丢弃,仍是旧 Loaded
        update_with(
            &mut ws,
            Message::ColumnsLoaded {
                project_id: 42,
                source_id: "s1".into(),
                schema: Some("public".into()),
                table: "users".into(),
                result: Ok(vec![]),
            },
        );
        assert!(matches!(
            ws.schema_state("s1").unwrap().column_load(&key),
            Some(ColumnLoad::Loaded(cols)) if cols.len() == 1
        ));
    }

    #[test]
    fn failed_columns_retry_by_recollapse_expand() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        update_with(&mut ws, Message::ToggleSchema("s1".into(), "public".into()));
        let key = (Some("public".to_string()), "users".to_string());
        update_with(
            &mut ws,
            Message::ToggleTable {
                source_id: "s1".into(),
                schema: key.0.clone(),
                table: key.1.clone(),
            },
        );
        update_with(
            &mut ws,
            Message::ColumnsLoaded {
                project_id: 42,
                source_id: "s1".into(),
                schema: key.0.clone(),
                table: key.1.clone(),
                result: Err("boom".into()),
            },
        );
        assert!(
            tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::ColumnsFailed("boom")))
        );
        // 收起再展开 → 回到 Loading(重试语义)
        update_with(
            &mut ws,
            Message::ToggleTable {
                source_id: "s1".into(),
                schema: key.0.clone(),
                table: key.1.clone(),
            },
        );
        update_with(
            &mut ws,
            Message::ToggleTable {
                source_id: "s1".into(),
                schema: key.0.clone(),
                table: key.1.clone(),
            },
        );
        assert!(matches!(
            ws.schema_state("s1").unwrap().column_load(&key),
            Some(ColumnLoad::Loading)
        ));
    }

    #[test]
    fn tables_loaded_ok_prunes_autoexpands_and_reloads_expanded() {
        let mut ws = seeded_ws(pg_source("s1"));
        {
            let st = ws.schemas.get_mut("s1").unwrap();
            st.loading_tables = true;
            st.tables = vec![
                tree_table(Some("public"), "gone", false),
                tree_table(Some("public"), "users", false),
            ];
            st.expanded_tables
                .insert((Some("public".into()), "users".into()));
            st.columns.insert(
                (Some("public".into()), "users".into()),
                ColumnLoad::Failed("旧错误".into()),
            );
            st.expanded_tables
                .insert((Some("public".into()), "gone".into()));
        }
        update_with(
            &mut ws,
            Message::TablesLoaded(
                42,
                "s1".into(),
                Ok(vec![tree_table(Some("public"), "users", false)]),
            ),
        );
        let st = ws.schema_state("s1").unwrap();
        assert!(!st.loading_tables());
        assert_eq!(st.tables().len(), 1);
        // 对账一:gone 的展开项被清
        let rows = tree_rows(st, DriverKind::Postgres);
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(t) if t.name == "gone"))
        );
        // 对账二:单 schema public 自动展开(users 直接可见)
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(t) if t.name == "users"))
        );
        // 对账三:仍展开的 users 重新拉列(Failed → Loading)
        assert!(matches!(
            st.column_load(&(Some("public".into()), "users".into())),
            Some(ColumnLoad::Loading)
        ));
    }

    #[test]
    fn tables_loaded_stale_or_err() {
        // 过期:loading_tables=false → 丢弃
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "keep", false)];
        update_with(
            &mut ws,
            Message::TablesLoaded(
                42,
                "s1".into(),
                Ok(vec![tree_table(Some("public"), "new", false)]),
            ),
        );
        assert_eq!(ws.schema_state("s1").unwrap().tables()[0].name, "keep");

        // 错误:记 tables_error,旧快照保留
        let mut ws = seeded_ws(pg_source("s2"));
        let st = ws.schemas.get_mut("s2").unwrap();
        st.loading_tables = true;
        st.tables = vec![tree_table(Some("public"), "old", false)];
        update_with(
            &mut ws,
            Message::TablesLoaded(42, "s2".into(), Err("net down".into())),
        );
        let st = ws.schema_state("s2").unwrap();
        assert_eq!(st.tables_error(), Some("net down"));
        assert_eq!(st.tables()[0].name, "old"); // 旧快照不闪空
    }

    #[test]
    fn delete_source_and_draft_save_clear_schema_state() {
        // DeleteSource:正在浏览的源被删 → schemas 条目与 browsing 一并清掉
        let mut ws = seeded_ws(pg_source("s1"));
        update_with(&mut ws, Message::DeleteSource("s1".into()));
        assert!(ws.schema_state("s1").is_none());
        assert!(!ws.is_expanded("s1"));

        // DraftSave 编辑已有源 → 同样清理(连接信息可能变,旧快照不作数)
        let mut ws = seeded_ws(pg_source("s1"));
        ws.editing = Some(DataSourceDraft {
            id: Some("s1".into()),
            name: "renamed".into(),
            driver: DriverKind::Postgres,
            host: "127.0.0.1".into(),
            port: "1".into(),
            database: "mydb".into(),
            username: "user".into(),
            uri: String::new(),
            password: String::new(), // 留空 → 跳过 Keychain,不写真实密码
        });
        update_with(&mut ws, Message::DraftSave);
        assert!(ws.schema_state("s1").is_none());
        assert!(!ws.is_expanded("s1"));
    }

    #[test]
    fn draft_save_legal_uri_redacts_and_backfills_fields() {
        // 无密码 URI:不触 Keychain(本仓库测试不写真实钥匙串)。
        let draft = DataSourceDraft {
            id: None, // 新建
            name: "cloud".into(),
            driver: DriverKind::Postgres,
            host: "wrong-field-host".into(), // 字段填错,应为 URI 覆盖
            port: String::new(),
            database: String::new(),
            username: String::new(),
            uri: "postgresql://alice@db:5433/shop".into(),
            password: String::new(),
        };
        let mut ws = WorkspaceState {
            editing: Some(draft),
            ..Default::default()
        };
        update_with(&mut ws, Message::DraftSave);
        let src = ws.sources.iter().find(|s| s.name == "cloud").unwrap();
        assert_eq!(src.uri.as_deref(), Some("postgresql://alice@db:5433/shop"));
        assert_eq!(src.host.as_deref(), Some("db")); // URI 优先,覆盖错误字段
        assert_eq!(src.port, Some(5433));
        assert_eq!(src.database.as_deref(), Some("shop"));
        assert_eq!(src.username.as_deref(), Some("alice"));
        // 无密码 → 不产生 Keychain 写入,uri 无明文密码(来源本就没有)。
        assert_eq!(src.uri.as_deref(), Some("postgresql://alice@db:5433/shop"));
    }

    #[test]
    fn draft_save_invalid_uri_falls_back_to_fields() {
        let draft = DataSourceDraft {
            id: None,
            name: "legacy".into(),
            driver: DriverKind::Postgres,
            host: "plain.example".into(),
            port: "6543".into(),
            database: "mydb".into(),
            username: "user".into(),
            uri: "https://not-a-db-scheme".into(), // 非法 → 退化为字段式
            password: String::new(),
        };
        let mut ws = WorkspaceState {
            editing: Some(draft),
            ..Default::default()
        };
        update_with(&mut ws, Message::DraftSave);
        let src = ws.sources.iter().find(|s| s.name == "legacy").unwrap();
        assert_eq!(src.uri, None); // 非法 URI 不落盘
        assert_eq!(src.host.as_deref(), Some("plain.example"));
        assert_eq!(src.port, Some(6543));
        assert_eq!(src.database.as_deref(), Some("mydb"));
        assert_eq!(src.username.as_deref(), Some("user"));
    }
}

#[cfg(test)]
mod sqlite_introspection {
    use super::*;
    use std::sync::Once;

    static INSTALL: Once = Once::new();

    fn install_drivers() {
        INSTALL.call_once(sqlx::any::install_default_drivers);
    }

    async fn setup_db() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        // mode=rwc:文件不存在时创建(默认 create_if_missing=false,新库必须显式造文件)
        let url = format!("sqlite://{}/phase2.sqlite?mode=rwc", dir.path().display());
        let pool = sqlx::AnyPool::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE VIEW v_users AS SELECT id FROM users")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        (dir, url)
    }

    #[tokio::test]
    async fn load_tables_lists_tables_and_views_in_order() {
        install_drivers();
        let (_dir, url) = setup_db().await;
        let tables = load_tables(DriverKind::Sqlite, &url, None).await.unwrap();
        assert_eq!(tables.len(), 2);
        // SQLite: name ORDER BY → users 在 v_users 前
        assert_eq!(tables[0].name, "users");
        assert!(!tables[0].is_view);
        assert_eq!(tables[1].name, "v_users");
        assert!(tables[1].is_view);
        assert!(tables.iter().all(|t| t.schema.is_none()));
    }

    #[tokio::test]
    async fn load_columns_reports_name_type_nullability() {
        install_drivers();
        let (_dir, url) = setup_db().await;
        let cols = load_columns(DriverKind::Sqlite, &url, None, "users")
            .await
            .unwrap();
        assert_eq!(cols.len(), 2);
        // pragma_table_info 按 cid 序:id 在前
        assert_eq!(cols[0].name, "id");
        let name = &cols[1];
        assert_eq!(name.name, "name");
        assert_eq!(name.type_name.to_uppercase(), "TEXT");
        assert!(!name.nullable); // NOT NULL 列
    }

    #[tokio::test]
    async fn load_columns_missing_table_is_error() {
        install_drivers();
        let (_dir, url) = setup_db().await;
        let err = load_columns(DriverKind::Sqlite, &url, None, "nope")
            .await
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn mongodb_loaders_refuse_with_friendly_error() {
        // 缺数据库名 → 友好文案,不 panic/不尝试连接
        let err = load_tables(DriverKind::MongoDB, "mongodb://x", None)
            .await
            .unwrap_err();
        assert!(err.contains("数据库名"));
        let err = load_columns(DriverKind::MongoDB, "mongodb://x", None, "c")
            .await
            .unwrap_err();
        assert!(err.contains("MongoDB"));
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;

    #[test]
    fn open_table_dedups_and_focuses_existing() {
        let mut st = DatabaseContentState::default();
        let id1 = st.open_table("s1".into(), Some("public".into()), "users".into());
        st.open_table("s1".into(), Some("public".into()), "orders".into());
        assert_eq!(st.tabs().len(), 2);
        let id_again = st.open_table("s1".into(), Some("public".into()), "users".into());
        assert_eq!(id1, id_again);
        assert_eq!(st.tabs().len(), 2); // 没有新开
        assert_eq!(st.active_idx(), Some(0)); // 聚焦回第一个 tab
    }

    #[test]
    fn open_table_reopen_preserves_content() {
        let mut st = DatabaseContentState::default();
        let id = st.open_table("s1".into(), None, "users".into());
        if let Some(TabContent::Browse(b)) = st.content_mut(id) {
            b.where_clause = "id > 10".into();
        }
        st.open_collection("s1".into(), "other".into()); // 切走
        st.open_table("s1".into(), None, "users".into()); // 再开同一张表
        let TabContent::Browse(b) = st.content(id).unwrap() else {
            panic!("应为 Browse");
        };
        assert_eq!(b.where_clause, "id > 10"); // 内容没被重置
    }

    #[test]
    fn open_collection_dedups() {
        let mut st = DatabaseContentState::default();
        let id1 = st.open_collection("s1".into(), "logs".into());
        let id2 = st.open_collection("s1".into(), "logs".into());
        assert_eq!(id1, id2);
        assert_eq!(st.tabs().len(), 1);
    }

    #[test]
    fn open_query_never_dedups_and_increments_seq() {
        let mut st = DatabaseContentState::default();
        st.open_query("s1".into());
        st.open_query("s1".into());
        assert_eq!(st.tabs().len(), 2);
        let seqs: Vec<u32> = st
            .tabs()
            .iter()
            .map(|t| match &t.kind {
                DatabaseTabKind::Query { console_seq, .. } => *console_seq,
                _ => panic!("应为 Query"),
            })
            .collect();
        assert_eq!(seqs, vec![1, 2]);
    }

    #[test]
    fn close_before_active_shifts_active_index_down() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.open_table("s1".into(), None, "b".into());
        st.select(1); // active = b(索引1)
        st.close(0); // 关掉 a(在 active 之前)
        assert_eq!(st.tabs().len(), 1);
        assert_eq!(st.active_idx(), Some(0)); // 仍指向 b,现在挪到索引0
        assert_eq!(
            st.tabs()[0].kind,
            DatabaseTabKind::Table {
                source_id: "s1".into(),
                schema: None,
                table: "b".into()
            }
        );
    }

    #[test]
    fn close_active_last_tab_shrinks_active() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.open_table("s1".into(), None, "b".into());
        // active 目前是索引1(b,刚开的)
        st.close(1);
        assert_eq!(st.tabs().len(), 1);
        assert_eq!(st.active_idx(), Some(0));
    }

    #[test]
    fn close_only_tab_empties_state() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.close(0);
        assert!(st.tabs().is_empty());
        assert!(st.active_tab().is_none());
    }

    #[test]
    fn close_by_source_removes_all_kinds_for_that_source() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.open_collection("s1".into(), "c".into());
        st.open_query("s1".into());
        st.open_table("s2".into(), None, "keep".into());
        st.close_by_source("s1");
        assert_eq!(st.tabs().len(), 1);
        assert_eq!(
            st.tabs()[0].kind,
            DatabaseTabKind::Table {
                source_id: "s2".into(),
                schema: None,
                table: "keep".into()
            }
        );
    }
}

#[cfg(test)]
mod query_gen_tests {
    use super::*;

    #[test]
    fn quote_table_postgres_with_schema() {
        assert_eq!(
            quote_table(DriverKind::Postgres, Some("public"), "users"),
            "\"public\".\"users\""
        );
    }

    #[test]
    fn quote_table_mysql_and_sqlite_use_backticks() {
        assert_eq!(quote_table(DriverKind::MySQL, None, "users"), "`users`");
        assert_eq!(quote_table(DriverKind::Sqlite, None, "users"), "`users`");
    }

    #[test]
    fn build_browse_sql_omits_empty_where_and_order_by() {
        let sql = build_browse_sql(DriverKind::Sqlite, None, "users", "", "", 0, 50);
        assert_eq!(sql, "SELECT * FROM `users` LIMIT 51 OFFSET 0");
    }

    #[test]
    fn build_browse_sql_includes_where_and_order_by_and_pages() {
        let sql = build_browse_sql(
            DriverKind::Postgres,
            Some("public"),
            "users",
            "id > 10",
            "id DESC",
            2,
            50,
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"public\".\"users\" WHERE id > 10 ORDER BY id DESC LIMIT 51 OFFSET 100"
        );
    }

    #[test]
    fn classify_statement_recognizes_row_producing_keywords() {
        for sql in [
            "select 1",
            "  SELECT * FROM t",
            "with x as (select 1) select * from x",
            "SHOW TABLES",
            "explain select 1",
            "pragma table_info(t)",
        ] {
            assert_eq!(classify_statement(sql), StatementKind::Rows, "sql={sql}");
        }
    }

    #[test]
    fn classify_statement_treats_dml_ddl_as_execute() {
        for sql in [
            "insert into t values (1)",
            "UPDATE t SET x=1",
            "delete from t",
            "CREATE TABLE t (id int)",
        ] {
            assert_eq!(classify_statement(sql), StatementKind::Execute, "sql={sql}");
        }
    }

    #[test]
    fn is_ddl_keyword_matches_only_ddl() {
        assert!(is_ddl_keyword("CREATE TABLE t (id int)"));
        assert!(is_ddl_keyword("  drop table t"));
        assert!(!is_ddl_keyword("insert into t values (1)"));
        assert!(!is_ddl_keyword("update t set x=1"));
    }
}

#[cfg(test)]
mod sqlite_browse_and_query {
    use super::*;

    async fn setup_db_with_rows() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/browse.sqlite?mode=rwc", dir.path().display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        for i in 1..=5 {
            sqlx::query("INSERT INTO items (id, name, note) VALUES (?, ?, ?)")
                .bind(i)
                .bind(format!("item-{i}"))
                .bind(if i == 3 {
                    None::<String>
                } else {
                    Some(format!("note-{i}"))
                })
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
        (dir, url)
    }

    #[tokio::test]
    async fn browse_table_paginates_and_reports_has_more() {
        let (_dir, url) = setup_db_with_rows().await;
        let page0 = browse_table(BrowseTableParams {
            kind: DriverKind::Sqlite,
            url: &url,
            schema: None,
            table: "items",
            where_clause: "",
            order_by: "id ASC",
            page: 0,
            page_size: 2,
        })
        .await
        .unwrap();
        assert_eq!(page0.result.columns, vec!["id", "name", "note"]);
        assert_eq!(page0.result.rows.len(), 2);
        assert!(page0.has_more);

        let page2 = browse_table(BrowseTableParams {
            kind: DriverKind::Sqlite,
            url: &url,
            schema: None,
            table: "items",
            where_clause: "",
            order_by: "id ASC",
            page: 2,
            page_size: 2,
        })
        .await
        .unwrap();
        assert_eq!(page2.result.rows.len(), 1); // 第5条,最后一页
        assert!(!page2.has_more);
    }

    #[tokio::test]
    async fn browse_table_where_clause_filters_and_null_renders_as_null() {
        let (_dir, url) = setup_db_with_rows().await;
        let page = browse_table(BrowseTableParams {
            kind: DriverKind::Sqlite,
            url: &url,
            schema: None,
            table: "items",
            where_clause: "id = 3",
            order_by: "",
            page: 0,
            page_size: 50,
        })
        .await
        .unwrap();
        assert_eq!(page.result.rows.len(), 1);
        let note_idx = page
            .result
            .columns
            .iter()
            .position(|c| c == "note")
            .unwrap();
        assert_eq!(page.result.rows[0][note_idx], CellValue::Null);
    }

    #[tokio::test]
    async fn run_query_select_returns_rows() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(
            DriverKind::Sqlite,
            &url,
            "SELECT id, name FROM items WHERE id <= 2 ORDER BY id",
        )
        .await
        .unwrap();
        let QueryOutcome::Rows(result) = outcome else {
            panic!("应为 Rows");
        };
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.columns, vec!["id", "name"]);
    }

    #[tokio::test]
    async fn run_query_update_returns_affected_count() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(
            DriverKind::Sqlite,
            &url,
            "UPDATE items SET note = 'x' WHERE id <= 2",
        )
        .await
        .unwrap();
        assert!(matches!(outcome, QueryOutcome::Affected(2)));
    }

    #[tokio::test]
    async fn run_query_delete_returns_affected_count() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(DriverKind::Sqlite, &url, "DELETE FROM items WHERE id >= 4")
            .await
            .unwrap();
        assert!(matches!(outcome, QueryOutcome::Affected(2)));
    }

    #[tokio::test]
    async fn browse_real_and_datetime_values_render_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/types.sqlite?mode=rwc", dir.path().display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE t (score REAL, seen DATETIME, note TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t (score, seen, note) VALUES (3.25, '2026-01-02 03:04:05', 'x')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let page = browse_table(BrowseTableParams {
            kind: DriverKind::Sqlite,
            url: &url,
            schema: None,
            table: "t",
            where_clause: "",
            order_by: "",
            page: 0,
            page_size: 50,
        })
        .await
        .unwrap();
        // 真实数值/时间不是"不支持的类型":SQLite 路径统一被 stringify 成文本。
        // 行序尽力按数值/文本值摆列,这里只校验两个非 NULL 单元格写出了
        // 非空、非降级标记的文本(具体格式交给 sqlite 的 ToSql/stringify)。
        for cell in &page.result.rows[0] {
            match cell {
                CellValue::Text(s) => assert!(!s.is_empty()),
                CellValue::Null => panic!("行里所有列都插了值,不应有 NULL"),
            }
        }
        assert_eq!(page.result.rows[0].len(), 3);
    }

    #[tokio::test]
    async fn run_query_create_table_returns_ddl_outcome() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(DriverKind::Sqlite, &url, "CREATE TABLE extra (id INTEGER)")
            .await
            .unwrap();
        assert!(matches!(outcome, QueryOutcome::Ddl));
    }

    #[tokio::test]
    async fn run_query_syntax_error_is_reported() {
        let (_dir, url) = setup_db_with_rows().await;
        let err = run_query(DriverKind::Sqlite, &url, "SELEKT * FROM items")
            .await
            .unwrap_err();
        assert!(!err.is_empty());
    }
}

#[cfg(test)]
mod mongo_collection_tests {
    use super::*;

    #[tokio::test]
    async fn load_tables_mongo_without_db_name_gives_friendly_error() {
        let err = load_tables(DriverKind::MongoDB, "mongodb://x", None)
            .await
            .unwrap_err();
        assert!(err.contains("数据库名"));
    }

    #[tokio::test]
    async fn load_tables_mongo_with_empty_db_name_also_errors() {
        let err = load_tables(DriverKind::MongoDB, "mongodb://x", Some(""))
            .await
            .unwrap_err();
        assert!(err.contains("数据库名"));
    }
}

#[cfg(test)]
mod content_message_tests {
    use super::*;

    fn sqlite_source(id: &str) -> DataSource {
        DataSource {
            id: id.into(),
            name: format!("test-{id}"),
            driver: DriverKind::Sqlite,
            host: None,
            port: None,
            database: Some("/nonexistent/does-not-matter.sqlite".into()),
            username: None,
            uri: None,
        }
    }

    fn update_with(ws: &mut WorkspaceState, msg: Message) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut app_state = AppState::default();
        let dir = tempfile::tempdir().unwrap();
        rt.block_on(async {
            update(ws, &mut app_state, msg, 42, dir.path(), rt.handle(), |_| ());
        });
    }

    #[test]
    fn open_table_tab_message_creates_tab() {
        let mut ws = WorkspaceState::default();
        update_with(
            &mut ws,
            Message::OpenTableTab {
                source_id: "s1".into(),
                schema: None,
                table: "users".into(),
            },
        );
        assert_eq!(ws.content().tabs().len(), 1);
    }

    #[test]
    fn browse_where_changed_updates_state() {
        let mut ws = WorkspaceState::default();
        update_with(
            &mut ws,
            Message::OpenTableTab {
                source_id: "s1".into(),
                schema: None,
                table: "users".into(),
            },
        );
        let tab_id = ws.content().tabs()[0].id;
        update_with(
            &mut ws,
            Message::BrowseWhereChanged(tab_id, "id > 1".into()),
        );
        let TabContent::Browse(b) = ws.content().content(tab_id).unwrap() else {
            panic!("应为 Browse");
        };
        assert_eq!(b.where_clause, "id > 1");
    }

    #[test]
    fn close_tab_message_removes_it() {
        let mut ws = WorkspaceState::default();
        update_with(
            &mut ws,
            Message::OpenTableTab {
                source_id: "s1".into(),
                schema: None,
                table: "users".into(),
            },
        );
        update_with(&mut ws, Message::CloseTab(0));
        assert!(ws.content().tabs().is_empty());
    }

    #[test]
    fn delete_source_closes_its_tabs() {
        let mut ws = WorkspaceState::default();
        ws.sources.push(sqlite_source("s1"));
        update_with(
            &mut ws,
            Message::OpenTableTab {
                source_id: "s1".into(),
                schema: None,
                table: "users".into(),
            },
        );
        assert_eq!(ws.content().tabs().len(), 1);
        update_with(&mut ws, Message::DeleteSource("s1".into()));
        assert!(ws.content().tabs().is_empty());
    }

    #[test]
    fn apply_browse_result_ok_and_err_paths() {
        let mut b = BrowseState::default();
        b.begin_run();
        apply_browse_result(
            &mut b,
            Ok(BrowsePage {
                result: QueryResult {
                    columns: vec!["id".into()],
                    rows: vec![vec![CellValue::Text("1".into())]],
                },
                has_more: true,
            }),
        );
        assert!(!b.loading);
        assert!(b.has_more);
        assert_eq!(b.result.as_ref().unwrap().rows.len(), 1);

        let mut b2 = BrowseState::default();
        b2.begin_run();
        apply_browse_result(&mut b2, Err("boom".into()));
        assert_eq!(b2.error.as_deref(), Some("boom"));
    }

    /// 过期的浏览结果(seq 不匹配当前轮)在 `update()` 的 `BrowseResult` 臂里
    /// 被丢弃,不落地——对应人工验收里"连续快速改 WHERE 各回车一次,最终停
    /// 在最后一次结果"的竞态防线(`run_seq`)。
    #[test]
    fn browse_result_stale_or_unloading_seq_is_dropped() {
        let mut ws = WorkspaceState::default();
        let tab = Message::OpenTableTab {
            source_id: "s1".into(),
            schema: None,
            table: "users".into(),
        };
        update_with(&mut ws, tab);
        let tab_id = ws.content().tabs()[0].id;
        // tab 打开后还没有任何 run:`loading=false`,seq 进来对不上 → 弃。
        let page = BrowsePage {
            result: QueryResult {
                columns: vec!["id".into()],
                rows: vec![vec![CellValue::Text("1".into())]],
            },
            has_more: false,
        };
        update_with(&mut ws, Message::BrowseResult(42, tab_id, 7, Ok(page)));
        let TabContent::Browse(b) = ws.content().content(tab_id).unwrap() else {
            panic!("应为 Browse");
        };
        assert!(b.result.is_none(), "过期/未运行的结果不应落地");
        assert!(!b.loading);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tab_overflow_items_empty_when_no_tabs_open() {
        let ws_state = WorkspaceState::default();
        assert!(tab_overflow_items(&ws_state).is_empty());
    }
}
