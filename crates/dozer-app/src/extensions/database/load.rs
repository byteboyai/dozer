//! Database 面板的 SQL 与加载:表/列清单 SQL、异步加载与 spawn、驱动/路径/
//! 数据源文件读写。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::*;

/// 表/视图清单 SQL(免绑定参数;三种后端各写各的,is_view 在 Rust 侧判断)。
pub(crate) fn tables_sql(driver: DriverKind) -> &'static str {
    match driver {
        DriverKind::Postgres => {
            // information_schema.tables 列是 sql_identifier/character_data(sqlx Any 驱动
            // 只认 text/varchar),必须 ::text 转成 text 才不会被 Any 拒绝解码。
            "SELECT table_schema::text, table_name::text, table_type::text \
             FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema') \
             ORDER BY table_schema, table_name"
        }
        DriverKind::MySQL => {
            "SELECT table_name, table_type \
             FROM information_schema.tables \
             WHERE table_schema = DATABASE() \
             ORDER BY table_name"
        }
        DriverKind::Sqlite => {
            "SELECT name, type FROM sqlite_master \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
             ORDER BY name"
        }
        DriverKind::MongoDB => {
            unreachable!("tables_sql 不处理 MongoDB(UI 无入口 + load_tables 双保险)")
        }
    }
}

/// 列清单 SQL(需绑定参数 → 占位符风格按后端)。
pub(crate) fn columns_sql(driver: DriverKind) -> &'static str {
    match driver {
        DriverKind::Postgres => {
            // 同 tables:@Any 不认 sql_identifier/character_data,列首三列 ::text 转 text
            "SELECT column_name::text, data_type::text, is_nullable::text \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position"
        }
        DriverKind::MySQL => {
            "SELECT column_name, data_type, is_nullable \
             FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = ? \
             ORDER BY ordinal_position"
        }
        // pragma_table_info 表值函数:常规 SELECT,`Any` 驱动下比 PRAGMA 语句稳;
        // notnull 是保留字,须用 "notnull" 引号包住(SQLite 3.x 实库验证)
        DriverKind::Sqlite => {
            "SELECT name, type, \"notnull\" FROM pragma_table_info(?) ORDER BY cid"
        }
        DriverKind::MongoDB => unreachable!("columns_sql 不处理 MongoDB"),
    }
}

pub(crate) async fn load_tables(
    kind: DriverKind,
    url: &str,
    mongo_db: Option<&str>,
) -> Result<Vec<TableRef>, String> {
    if kind == DriverKind::MongoDB {
        let Some(db_name) = mongo_db.filter(|s| !s.is_empty()) else {
            return Err("请在数据源配置里填写数据库名后再浏览集合".to_string());
        };
        return load_mongo_collections(url, db_name).await;
    }
    let work = async {
        let pool = sqlx::AnyPool::connect(url)
            .await
            .map_err(|e| e.to_string())?;
        let out = async {
            let sql = tables_sql(kind);
            if kind == DriverKind::Postgres {
                let rows: Vec<(String, String, String)> = sqlx::query_as(sql)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(
                    rows.into_iter()
                        .map(|(schema, name, table_type)| TableRef {
                            schema: Some(schema),
                            name,
                            is_view: table_type == "VIEW",
                        })
                        .collect(),
                )
            } else {
                let rows: Vec<(String, String)> = sqlx::query_as(sql)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(rows
                    .into_iter()
                    .map(|(name, ty)| TableRef {
                        schema: None,
                        name,
                        // MySQL: information_schema.tables.table_type == 'VIEW';
                        // SQLite: sqlite_master.type == 'view'
                        is_view: if kind == DriverKind::MySQL {
                            ty == "VIEW"
                        } else {
                            ty == "view"
                        },
                    })
                    .collect())
            }
        }
        .await;
        pool.close().await;
        out
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), work).await {
        Ok(r) => r,
        Err(_) => Err("加载超时(5秒)".to_string()),
    }
}

/// MongoDB 集合清单,伪装成 `TableRef`(`schema: None, is_view: false`)——
/// 复用阶段 2 现成的树渲染/展开数据结构,不新开 `SchemaRowKind` 变体
/// (设计文档写作时预留了 `SchemaRowKind::Collection` 的可能性,写计划时
/// 发现集合的"形状"和 MySQL/SQLite 的表完全一致,真正的差异只在**点击行为**
/// ——这个差异在 Task 7 的 `schema_tree_row` 里按 `driver` 参数处理,不需要
/// 数据模型层面的新类型)。
pub(crate) async fn load_mongo_collections(
    url: &str,
    db_name: &str,
) -> Result<Vec<TableRef>, String> {
    let work = async {
        let client = mongodb::Client::with_uri_str(url)
            .await
            .map_err(|e| e.to_string())?;
        let names = client
            .database(db_name)
            .list_collection_names()
            .await
            .map_err(|e| e.to_string())?;
        let mut refs: Vec<TableRef> = names
            .into_iter()
            .map(|name| TableRef {
                schema: None,
                name,
                is_view: false,
            })
            .collect();
        refs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok::<_, String>(refs)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), work).await {
        Ok(r) => r,
        Err(_) => Err("加载超时(5秒)".to_string()),
    }
}

pub(crate) async fn load_columns(
    kind: DriverKind,
    url: &str,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    if kind == DriverKind::MongoDB {
        return Err("MongoDB 集合浏览将在后续阶段支持".to_string());
    }
    let work = async {
        let pool = sqlx::AnyPool::connect(url)
            .await
            .map_err(|e| e.to_string())?;
        let out = async {
            let out: Vec<ColumnInfo> = match kind {
                DriverKind::Postgres => {
                    let rows: Vec<(String, String, String)> = sqlx::query_as(columns_sql(kind))
                        .bind(schema.unwrap_or("public"))
                        .bind(table)
                        .fetch_all(&pool)
                        .await
                        .map_err(|e| e.to_string())?;
                    rows.into_iter()
                        .map(|(name, dt, nullable)| ColumnInfo {
                            name,
                            type_name: dt,
                            nullable: nullable == "YES",
                        })
                        .collect()
                }
                DriverKind::MySQL => {
                    let rows: Vec<(String, String, String)> = sqlx::query_as(columns_sql(kind))
                        .bind(table)
                        .fetch_all(&pool)
                        .await
                        .map_err(|e| e.to_string())?;
                    rows.into_iter()
                        .map(|(name, dt, nullable)| ColumnInfo {
                            name,
                            type_name: dt,
                            nullable: nullable == "YES",
                        })
                        .collect()
                }
                DriverKind::Sqlite => {
                    let rows: Vec<(String, String, i64)> = sqlx::query_as(columns_sql(kind))
                        .bind(table)
                        .fetch_all(&pool)
                        .await
                        .map_err(|e| e.to_string())?;
                    rows.into_iter()
                        .map(|(name, ty, notnull)| ColumnInfo {
                            name,
                            type_name: ty,
                            nullable: notnull == 0,
                        })
                        .collect()
                }
                DriverKind::MongoDB => unreachable!("load_columns 已在入口拦下 MongoDB"),
            };
            // 真实表必然有 ≥1 列;0 行 → 表不存在(Postgres/MySQL 走 information_schema,
            // SQLite 走 pragma_table_info,缺表一律返回空集而非错误)
            if out.is_empty() {
                return Err(format!("表不存在或无可见列:{table}"));
            }
            Ok::<_, String>(out)
        }
        .await;
        pool.close().await;
        out
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), work).await {
        Ok(r) => r,
        Err(_) => Err("加载超时(5秒)".to_string()),
    }
}

pub(crate) fn spawn_tables_load(
    handle: &tokio::runtime::Handle,
    project_id: i64,
    source_id: String,
    kind: DriverKind,
    url: String,
    mongo_db: Option<String>,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let result = load_tables(kind, &url, mongo_db.as_deref()).await;
        emit(Message::TablesLoaded(project_id, source_id, result));
    });
}

#[allow(clippy::too_many_arguments)] // 计划给定签名:加载器游标 + 结果回传 emit 共 8 参,边界合理
pub(crate) fn spawn_columns_load(
    handle: &tokio::runtime::Handle,
    project_id: i64,
    source_id: String,
    kind: DriverKind,
    url: String,
    schema: Option<String>,
    table: String,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let result = load_columns(kind, &url, schema.as_deref(), &table).await;
        emit(Message::ColumnsLoaded {
            project_id,
            source_id,
            schema,
            table,
            result,
        });
    });
}

pub(crate) fn driver_of(ws_state: &WorkspaceState, source_id: &str) -> Option<DriverKind> {
    ws_state
        .sources
        .iter()
        .find(|s| s.id == source_id)
        .map(|s| s.driver)
}

pub(crate) fn drivers_path() -> PathBuf {
    dozer_core::paths::config_dir().join("database_drivers.json")
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EnabledDriversFile {
    pub(crate) enabled: Vec<DriverKind>,
}

pub fn sources_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("database.json")
}

/// 从 `.dozer/database.json` 重读数据源列表。文件不存在/格式损坏→空列表,
/// 不 panic(同 `todo`/`goal` 现有容错口径)。**不**清空 `test_status`/
/// `editing`——重读只影响 `sources` 本身,进行中的编辑/测试状态原样保留。
pub fn reload_from_disk(ws_state: &mut WorkspaceState, repo: &Path) {
    let path = sources_path(repo);
    ws_state.sources = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
}

pub(crate) fn save_sources(repo: &Path, sources: &[DataSource]) -> std::io::Result<()> {
    let dir = repo.join(".dozer");
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(sources).expect("Vec<DataSource> 总能序列化");
    std::fs::write(sources_path(repo), json)
}

/// Keychain 条目 key:`{project_id}:{source_id}`,同一把钥匙串条目按项目+
/// 数据源双重区分。
pub(crate) fn keyring_entry(
    project_id: i64,
    source_id: &str,
) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new("dozer", &format!("{project_id}:{source_id}"))
}
