//! Database 面板 update 消息分发 + 浏览/查询结果应用 + 连接串构造/解析/
//! 密码注入 + 连接测试。

use std::collections::HashSet;
use std::path::Path;

use super::*;

pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    repo_path: &Path,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Clone + 'static,
) {
    match msg {
        // 这两个溢出开关由 `app.rs` 主级 `update` 拦截(需要 `App::last_cursor`),
        // 正常不会走到这个子级 `database::update`——保留空 arm 只为满足穷尽。
        Message::TabOverflowToggle | Message::TabOverflowDismiss => {}
        Message::ToggleDriver(driver) => app_state.toggle(driver),
        Message::DriversPopupToggle => {
            app_state.drivers_popup_open = !app_state.drivers_popup_open;
        }
        Message::AddSourceStart => {
            ws_state.editing = Some(DataSourceDraft::default());
            ws_state.draft_test_status = TestStatus::Idle;
        }
        Message::EditSourceStart(id) => {
            ws_state.draft_test_status = TestStatus::Idle;
            if let Some(src) = ws_state.sources.iter().find(|s| s.id == id) {
                ws_state.editing = Some(DataSourceDraft {
                    id: Some(src.id.clone()),
                    name: src.name.clone(),
                    driver: src.driver,
                    host: src.host.clone().unwrap_or_default(),
                    port: src.port.map(|p| p.to_string()).unwrap_or_default(),
                    database: src.database.clone().unwrap_or_default(),
                    username: src.username.clone().unwrap_or_default(),
                    password: String::new(), // 不回显已存密码,留空=不改密码
                    uri: src.uri.clone().unwrap_or_default(),
                });
            }
        }
        Message::DraftNameChanged(v) => set_draft(ws_state, |d| d.name = v),
        Message::TextInputMenuOpen(_) => {
            unreachable!("由内核拦截处理,见 database::Message::TextInputMenuOpen 文档")
        }
        Message::ToggleListCollapse => {
            unreachable!("由内核拦截处理,见 database::Message::ToggleListCollapse 文档")
        }
        Message::Hover(_, _) => {
            unreachable!("由内核拦截处理,见 database::Message::Hover 文档")
        }
        Message::SourceContextMenu(_) => {
            unreachable!("由内核拦截处理,见 database::Message::SourceContextMenu 文档")
        }
        Message::DraftDriverChanged(v) => set_draft(ws_state, |d| d.driver = v),
        Message::DraftHostChanged(v) => set_draft(ws_state, |d| d.host = v),
        Message::DraftPortChanged(v) => set_draft(ws_state, |d| d.port = v),
        Message::DraftDatabaseChanged(v) => set_draft(ws_state, |d| d.database = v),
        Message::DraftUsernameChanged(v) => set_draft(ws_state, |d| d.username = v),
        Message::DraftPasswordChanged(v) => set_draft(ws_state, |d| d.password = v),
        Message::DraftUriChanged(v) => set_draft(ws_state, |d| d.uri = v),
        Message::DraftSave => {
            let Some(draft) = ws_state.editing.take() else {
                return;
            };
            if draft.name.trim().is_empty() {
                ws_state.editing = Some(draft); // 名字必填,打回表单
                return;
            }
            let id = draft
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let (source, pw_to_save) = draft_to_source(id.clone(), &draft);
            if let Some(pos) = ws_state.sources.iter().position(|s| s.id == id) {
                ws_state.sources[pos] = source;
            } else {
                ws_state.sources.push(source);
            }
            if let Some(p) = pw_to_save
                && let Ok(entry) = keyring_entry(project_id, &id)
            {
                let _ = entry.set_password(&p);
            }
            // 编辑已有源:连接信息可能变了,旧结构快照不作数(设计文档 §2)。
            if draft.id.is_some() {
                ws_state.schemas.remove(&id);
                ws_state.content.close_by_source(&id);
                ws_state.expanded_sources.remove(&id);
            }
            if let Err(e) = save_sources(repo_path, &ws_state.sources) {
                tracing::warn!("写入 database.json 失败: {e}");
            }
            ws_state.draft_test_status = TestStatus::Idle;
        }
        Message::DraftCancel => {
            ws_state.editing = None;
            ws_state.draft_test_status = TestStatus::Idle;
        }
        Message::DraftTestConnection => {
            let Some(draft) = ws_state.editing.clone() else {
                return;
            };
            let (source, mut password) =
                draft_to_source(draft.id.clone().unwrap_or_default(), &draft);
            // 密码留空 = "编辑已有源时不改密码"的约定(同 `DraftSave`);测试
            // 连接要用回 Keychain 里已保存的那份,否则编辑时清空过密码框
            // 就永远测不通,即使原密码其实没变。
            if password.is_none()
                && let Some(id) = &draft.id
                && let Ok(entry) = keyring_entry(project_id, id)
            {
                password = entry.get_password().ok();
            }
            ws_state.draft_test_status = TestStatus::Testing;
            handle.spawn(async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    test_connection(source, password),
                )
                .await;
                let result = match result {
                    Ok(r) => r,
                    Err(_) => Err("连接超时(5秒)".to_string()),
                };
                emit(Message::DraftTestConnectionResult(result));
            });
        }
        Message::DraftTestConnectionResult(result) => {
            ws_state.draft_test_status = match result {
                Ok(()) => TestStatus::Ok,
                Err(e) => TestStatus::Err(e),
            };
        }
        Message::DeleteSourceRequest(id) => ws_state.request_delete(id),
        Message::DeleteSourceCancel => ws_state.cancel_delete(),
        Message::DeleteSource(id) => {
            if ws_state.delete_confirm.as_deref() == Some(id.as_str()) {
                ws_state.delete_confirm = None;
            }
            ws_state.sources.retain(|s| s.id != id);
            ws_state.test_status.remove(&id);
            ws_state.schemas.remove(&id);
            ws_state.content.close_by_source(&id);
            ws_state.expanded_sources.remove(&id);
            if let Ok(entry) = keyring_entry(project_id, &id) {
                let _ = entry.delete_credential();
            }
            if let Err(e) = save_sources(repo_path, &ws_state.sources) {
                tracing::warn!("写入 database.json 失败: {e}");
            }
        }
        Message::TestConnection(id) => {
            let Some(source) = ws_state.sources.iter().find(|s| s.id == id).cloned() else {
                return;
            };
            ws_state.test_status.insert(id.clone(), TestStatus::Testing);
            let password = keyring_entry(project_id, &id)
                .ok()
                .and_then(|e| e.get_password().ok());
            handle.spawn(async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    test_connection(source, password),
                )
                .await;
                let result = match result {
                    Ok(r) => r,
                    Err(_) => Err("连接超时(5秒)".to_string()),
                };
                emit(Message::TestConnectionResult(project_id, id, result));
            });
        }
        Message::TestConnectionResult(_project_id, id, result) => {
            let status = match result {
                Ok(()) => TestStatus::Ok,
                Err(e) => TestStatus::Err(e),
            };
            ws_state.test_status.insert(id, status);
        }
        Message::ToggleSourceExpanded(id) => {
            if !ws_state.expanded_sources.remove(&id) {
                ws_state.expanded_sources.insert(id.clone());
            } else {
                return; // 收起:保留缓存,不取消飞行中的加载
            }
            let Some(source) = ws_state.sources.iter().find(|s| s.id == id).cloned() else {
                return;
            };
            let st = ws_state.schemas.entry(id.clone()).or_default();
            // 从未加载过才拉:有缓存 → 直接显示;有错误 → 等用户点右键菜单"刷新"
            let need_load = st.tables.is_empty() && !st.loading_tables && st.tables_error.is_none();
            if !need_load {
                return;
            }
            st.loading_tables = true;
            let password = keyring_entry(project_id, &id)
                .ok()
                .and_then(|e| e.get_password().ok());
            let url = if source.driver == DriverKind::MongoDB {
                build_mongo_url(&source, password.as_deref())
            } else {
                build_sql_url(&source, password.as_deref())
            };
            spawn_tables_load(
                handle,
                project_id,
                id,
                source.driver,
                url,
                source.database.clone(),
                emit,
            );
        }
        Message::SchemaRefresh(source_id) => {
            if !ws_state.expanded_sources.contains(&source_id) {
                return; // 菜单残留防线:源已被收起
            }
            let Some(source) = ws_state.sources.iter().find(|s| s.id == source_id).cloned() else {
                return;
            };
            let st = ws_state.schemas.entry(source_id.clone()).or_default();
            st.loading_tables = true;
            st.tables_error = None;
            // 旧表快照保留不闪空(设计文档 UI 节)
            let password = keyring_entry(project_id, &source_id)
                .ok()
                .and_then(|e| e.get_password().ok());
            let url = if source.driver == DriverKind::MongoDB {
                build_mongo_url(&source, password.as_deref())
            } else {
                build_sql_url(&source, password.as_deref())
            };
            spawn_tables_load(
                handle,
                project_id,
                source_id,
                source.driver,
                url,
                source.database.clone(),
                emit,
            );
        }
        Message::ToggleSchema(source_id, name) => {
            let Some(st) = ws_state.schemas.get_mut(&source_id) else {
                return;
            };
            if !st.expanded_schemas.remove(&name) {
                st.expanded_schemas.insert(name);
            }
        }
        Message::ToggleTable {
            source_id,
            schema,
            table,
        } => {
            if !ws_state.expanded_sources.contains(&source_id) {
                return; // 旧视图残留按钮防线
            }
            let Some(source) = ws_state.sources.iter().find(|s| s.id == source_id).cloned() else {
                return;
            };
            let key = (schema, table);
            let Some(st) = ws_state.schemas.get_mut(&source_id) else {
                return;
            };
            if st.expanded_tables.remove(&key) {
                return; // 收起:保留缓存,不取消飞行中的加载(回来照常写缓存)
            }
            st.expanded_tables.insert(key.clone());
            if matches!(
                st.columns.get(&key),
                Some(ColumnLoad::Loaded(_)) | Some(ColumnLoad::Loading)
            ) {
                return;
            }
            // 缺失或曾失败 → (重新)加载
            st.columns.insert(key.clone(), ColumnLoad::Loading);
            let password = keyring_entry(project_id, &source_id)
                .ok()
                .and_then(|e| e.get_password().ok());
            spawn_columns_load(
                handle,
                project_id,
                source.id.clone(),
                source.driver,
                build_sql_url(&source, password.as_deref()),
                key.0,
                key.1,
                emit,
            );
        }
        Message::TablesLoaded(_project_id, source_id, result) => {
            // 借用顺序:先只读查驱动,再拿 schemas 可变借用(st 存活期间
            // 不能再回借 ws_state.sources,见下 Ok 分支尾部)
            let is_pg = driver_of(ws_state, &source_id) == Some(DriverKind::Postgres);
            let Some(st) = ws_state.schemas.get_mut(&source_id) else {
                return;
            };
            if !st.loading_tables {
                return; // 过期防线:源已删/被新一轮加载覆盖
            }
            st.loading_tables = false;
            match result {
                Err(e) => {
                    st.tables_error = Some(e); // 旧快照保留,视图按有无快照分别渲染
                }
                Ok(tables) => {
                    st.tables_error = None;
                    st.tables = tables;
                    // 对账一:新表列表里不存在的展开项/列缓存清掉
                    let keys: HashSet<(Option<String>, String)> = st
                        .tables
                        .iter()
                        .map(|t| (t.schema.clone(), t.name.clone()))
                        .collect();
                    st.expanded_tables.retain(|k| keys.contains(k));
                    st.columns.retain(|k, _| keys.contains(k));
                    // 对账二:Postgres 全表同属一个 schema → 自动展开(MVP 少一次点击)
                    if is_pg {
                        let distinct: HashSet<&str> = st
                            .tables
                            .iter()
                            .filter_map(|t| t.schema.as_deref())
                            .collect();
                        if distinct.len() == 1
                            && let Some(only) = distinct.into_iter().next()
                        {
                            st.expanded_schemas.insert(only.to_string());
                        }
                    }
                    // 对账三:仍然展开的表全部重新拉列(置 Loading + spawn)
                    let reload: Vec<(Option<String>, String)> =
                        st.expanded_tables.iter().cloned().collect();
                    for key in &reload {
                        st.columns.insert(key.clone(), ColumnLoad::Loading);
                    }
                    let Some(source) = ws_state.sources.iter().find(|s| s.id == source_id).cloned()
                    else {
                        return;
                    };
                    for key in reload {
                        let password = keyring_entry(project_id, &source.id)
                            .ok()
                            .and_then(|e| e.get_password().ok());
                        spawn_columns_load(
                            handle,
                            project_id,
                            source.id.clone(),
                            source.driver,
                            build_sql_url(&source, password.as_deref()),
                            key.0,
                            key.1,
                            emit.clone(),
                        );
                    }
                }
            }
        }
        Message::ColumnsLoaded {
            project_id: _project_id,
            source_id,
            schema,
            table,
            result,
        } => {
            let Some(st) = ws_state.schemas.get_mut(&source_id) else {
                return;
            };
            let key = (schema, table);
            if !matches!(st.columns.get(&key), Some(ColumnLoad::Loading)) {
                return; // 过期防线:只接收仍在等的加载结果
            }
            st.columns.insert(
                key,
                match result {
                    Ok(cols) => ColumnLoad::Loaded(cols),
                    Err(e) => ColumnLoad::Failed(e),
                },
            );
        }
        Message::OpenTableTab {
            source_id,
            schema,
            table,
        } => {
            ws_state.content.open_table(source_id, schema, table);
        }
        Message::OpenCollectionTab { source_id, name } => {
            ws_state.content.open_collection(source_id, name);
        }
        Message::OpenQueryTab(source_id) => {
            ws_state.content.open_query(source_id);
        }
        Message::SelectTab(idx) => {
            ws_state.content.select(idx);
            reveal_tab_widths(ws_state, idx + 1);
        }
        Message::SelectBlankTab => {
            ws_state.content.select_blank();
            reveal_tab_widths(ws_state, 0);
        }
        Message::CloseTab(idx) => ws_state.content.close(idx),
        Message::BrowseWhereChanged(tab_id, v) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) {
                b.where_clause = v;
            }
        }
        Message::BrowseOrderByChanged(tab_id, v) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) {
                b.order_by = v;
            }
        }
        Message::BrowsePageSizeChanged(tab_id, size) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) {
                b.page_size = size;
                b.page = 0; // 换页大小回第一页,避免 offset 算错位置
            }
        }
        Message::BrowsePrev(tab_id) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id)
                && b.page > 0
            {
                b.page -= 1;
            }
            dispatch_browse_run(
                ws_state, app_state, tab_id, project_id, repo_path, handle, emit,
            );
        }
        Message::BrowseNext(tab_id) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id)
                && b.has_more
            {
                b.page += 1;
            }
            dispatch_browse_run(
                ws_state, app_state, tab_id, project_id, repo_path, handle, emit,
            );
        }
        Message::BrowseRun(tab_id) => {
            dispatch_browse_run(
                ws_state, app_state, tab_id, project_id, repo_path, handle, emit,
            );
        }
        Message::BrowseResult(_project_id, tab_id, seq, result) => {
            let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) else {
                return; // tab 已关闭
            };
            if !b.is_current_run(seq) {
                return; // 过期结果:tab 内又发起了更新的一轮请求,这轮作废
            }
            apply_browse_result(b, result);
        }
        Message::QueryTextAction(tab_id, action) => {
            if let Some(TabContent::Query(q)) = ws_state.content.content_mut(tab_id) {
                q.sql.perform(action);
            }
        }
        Message::QueryRun(tab_id) => {
            let Some(kind) = ws_state
                .content
                .tabs()
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| match &t.kind {
                    DatabaseTabKind::Query { source_id, .. } => driver_of(ws_state, source_id),
                    _ => None,
                })
            else {
                return;
            };
            let Some(source) = ws_state
                .content
                .tabs()
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| match &t.kind {
                    DatabaseTabKind::Query { source_id, .. } => ws_state
                        .sources
                        .iter()
                        .find(|s| &s.id == source_id)
                        .cloned(),
                    _ => None,
                })
            else {
                return;
            };
            let sql = match ws_state.content.content_mut(tab_id) {
                Some(TabContent::Query(q)) => {
                    let seq = q.begin_run();
                    let text = q.sql.text();
                    (seq, text)
                }
                _ => return,
            };
            let (seq, sql_text) = sql;
            let password = keyring_entry(project_id, &source.id)
                .ok()
                .and_then(|e| e.get_password().ok());
            let url = build_sql_url(&source, password.as_deref());
            let emit = emit.clone();
            handle.spawn(async move {
                let result = run_query(kind, &url, &sql_text).await;
                emit(Message::QueryResult(project_id, tab_id, seq, result));
            });
        }
        Message::QueryResult(_project_id, tab_id, seq, result) => {
            let Some(TabContent::Query(q)) = ws_state.content.content_mut(tab_id) else {
                return;
            };
            if !q.is_current_run(seq) {
                return; // 过期结果:同一 tab 内已经又执行了一次更新的查询
            }
            apply_query_result(q, result);
        }
        Message::TabHover(..) => {
            // 内容窗格 tab 悬停:由内核 `App::update` 的特化臂转发到
            // `HoverId::DatabaseTabItem/DatabaseTabClose`,吃不到这里。
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_browse_run(
    ws_state: &mut WorkspaceState,
    _app_state: &AppState,
    tab_id: usize,
    project_id: i64,
    repo_path: &Path,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Clone + 'static,
) {
    let Some(tab) = ws_state.content.tabs().iter().find(|t| t.id == tab_id) else {
        return;
    };
    let (source_id, mongo_name, table_schema, table_name) = match &tab.kind {
        DatabaseTabKind::Table {
            source_id,
            schema,
            table,
        } => (source_id.clone(), None, schema.clone(), Some(table.clone())),
        DatabaseTabKind::Collection { source_id, name } => {
            (source_id.clone(), Some(name.clone()), None, None)
        }
        DatabaseTabKind::Query { .. } => return, // 查询 tab 不走这条路径
    };
    let Some(source) = ws_state.sources.iter().find(|s| s.id == source_id).cloned() else {
        return;
    };
    let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) else {
        return;
    };
    let seq = b.begin_run();
    b.error = None;
    let (where_clause, order_by, page, page_size) = (
        b.where_clause.clone(),
        b.order_by.clone(),
        b.page,
        b.page_size,
    );
    let password = keyring_entry(project_id, &source.id)
        .ok()
        .and_then(|e| e.get_password().ok());
    let _ = repo_path; // 本函数不需要仓库路径,保留参数只为和 update() 里其它 dispatch 签名一致
    let emit2 = emit.clone();
    if source.driver == DriverKind::MongoDB {
        let url = build_mongo_url(&source, password.as_deref());
        let db_name = source.database.clone().unwrap_or_default();
        let name = mongo_name.unwrap_or_default();
        handle.spawn(async move {
            let result = browse_collection(&url, &db_name, &name, page, page_size).await;
            emit2(Message::BrowseResult(project_id, tab_id, seq, result));
        });
    } else {
        let url = build_sql_url(&source, password.as_deref());
        let kind = source.driver;
        let table = table_name.unwrap_or_default();
        handle.spawn(async move {
            let result = browse_table(BrowseTableParams {
                kind,
                url: &url,
                schema: table_schema.as_deref(),
                table: &table,
                where_clause: &where_clause,
                order_by: &order_by,
                page,
                page_size,
            })
            .await;
            emit2(Message::BrowseResult(project_id, tab_id, seq, result));
        });
    }
}

/// `BrowseResult` 落地:过期结果的核对(`run_seq` 对不上)在 `update()` 的
/// `Message::BrowseResult` 分支里做完才会调到这个函数,这里只管把结果写
/// 进状态。模块私有——`app.rs`(Task 6)不直接调它,而是把消息重新塞回
/// `database::update()`,由 `update()` 内部调这个函数。
pub(crate) fn apply_browse_result(b: &mut BrowseState, result: Result<BrowsePage, String>) {
    b.loading = false;
    match result {
        Ok(page) => {
            b.error = None;
            b.result = Some(page.result);
            b.has_more = page.has_more;
        }
        Err(e) => b.error = Some(e),
    }
}

/// `QueryResult` 落地,同上——模块私有,理由同 `apply_browse_result`。
pub(crate) fn apply_query_result(q: &mut QueryState, result: Result<QueryOutcome, String>) {
    q.running = false;
    match result {
        Ok(outcome) => {
            q.error = None;
            q.result = Some(outcome);
        }
        Err(e) => q.error = Some(e),
    }
}

pub(crate) fn set_draft(ws_state: &mut WorkspaceState, f: impl FnOnce(&mut DataSourceDraft)) {
    if let Some(draft) = ws_state.editing.as_mut() {
        f(draft);
    }
}

/// 表单草稿 → `DataSource` + 待用密码的统一转换:"连接 URI 优先于分字段,
/// URI 解析失败退化成分字段"这套逻辑,`DraftSave`(落盘)和
/// `DraftTestConnection`(测试连接,不落盘)两处共用,避免各写一份互相
/// 漂移。`id` 由调用方决定——`DraftSave` 传新 uuid 或已有 id(要落进
/// `ws_state.sources`);`DraftTestConnection` 随便传一个不落盘的临时值,
/// `DataSource.id` 本身不参与 `build_sql_url`/`build_mongo_url` 拼接,
/// 传什么都不影响连接结果。
pub(crate) fn draft_to_source(id: String, draft: &DataSourceDraft) -> (DataSource, Option<String>) {
    let mut uri: Option<String> = None;
    let mut uri_pw: Option<String> = None;
    let mut host = non_empty(&draft.host);
    let mut port = draft.port.parse().ok();
    let mut database = non_empty(&draft.database);
    let mut username = non_empty(&draft.username);
    // URI 无效(非 postgres/mysql 或解析失败):忽略 URI,退化为字段式。
    if !draft.uri.trim().is_empty()
        && let Some(p) = parse_connection_uri(draft.uri.trim())
    {
        uri = Some(p.uri);
        uri_pw = p.password;
        host = p.host;
        port = p.port;
        database = p.database;
        username = p.username;
    }
    let source = DataSource {
        id,
        name: draft.name.clone(),
        driver: draft.driver,
        host,
        port,
        database,
        username,
        uri,
    };
    // 密码:URI 内嵌优先,其次表单 password 字段。
    let password = uri_pw.or_else(|| {
        if draft.password.is_empty() {
            None
        } else {
            Some(draft.password.clone())
        }
    });
    (source, password)
}

pub(crate) fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub(crate) fn build_sql_url(source: &DataSource, password: Option<&str>) -> String {
    let scheme = match source.driver {
        DriverKind::Postgres => "postgres",
        DriverKind::MySQL => "mysql",
        DriverKind::Sqlite => "sqlite",
        DriverKind::MongoDB => unreachable!("build_sql_url 不处理 MongoDB"),
    };
    if source.driver == DriverKind::Sqlite {
        // SQLite: sqlite://<文件路径>,不带 host/port/用户名/密码。
        let path = source.database.as_deref().unwrap_or("");
        return format!("sqlite://{path}");
    }
    // 连接 URI 优先(用户整串粘贴);密码留 Keychain,用时补回。
    if let Some(u) = source.uri.as_deref().filter(|u| !u.trim().is_empty()) {
        let u = u.trim();
        if u.contains("://") {
            return inject_password_into_uri(u, password);
        }
        return u.to_string();
    }
    let host = source.host.as_deref().unwrap_or("localhost");
    let port = source.port.map(|p| format!(":{p}")).unwrap_or_default();
    let db = source.database.as_deref().unwrap_or("");
    let auth = match (source.username.as_deref(), password) {
        (Some(u), Some(p)) if !p.is_empty() => format!("{u}:{p}@"),
        (Some(u), _) => format!("{u}@"),
        (None, _) => String::new(),
    };
    format!("{scheme}://{auth}{host}{port}/{db}")
}

pub(crate) fn build_mongo_url(source: &DataSource, password: Option<&str>) -> String {
    let host = source.host.as_deref().unwrap_or("localhost");
    let port = source.port.map(|p| format!(":{p}")).unwrap_or_default();
    let auth = match (source.username.as_deref(), password) {
        (Some(u), Some(p)) if !p.is_empty() => format!("{u}:{p}@"),
        (Some(u), _) => format!("{u}@"),
        (None, _) => String::new(),
    };
    format!("mongodb://{auth}{host}{port}")
}

/// 解析用户粘贴的连接 URI(仅 postgres/postgresql/mysql)。返回:脱敏后的
/// 一次 `parse_connection_uri` 的解析结果。
pub(crate) struct ParsedUri {
    /// 脱敏后的 URI(密码已抽走,保留用户名/host/port/库名)。
    pub(crate) uri: String,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) database: Option<String>,
    pub(crate) username: Option<String>,
    /// 从 URI 抽出的密码,交调用方写入 Keychain。
    pub(crate) password: Option<String>,
}

/// 解析用户粘贴的连接 URI(仅 postgres/postgresql/mysql)。脱敏 = 密码抽走
/// (交调用方进 Keychain),URI 里不再留明文,避免写进 `database.json`。
/// 解析失败或 scheme 不受支持返回 None。
pub(crate) fn parse_connection_uri(uri: &str) -> Option<ParsedUri> {
    let u = url::Url::parse(uri).ok()?;
    let scheme = u.scheme();
    if scheme != "postgres" && scheme != "postgresql" && scheme != "mysql" {
        return None;
    }
    let host = u.host_str().map(|h| h.to_string());
    let host = host.filter(|h| !h.is_empty())?;
    let port = u.port();
    let database = u
        .path_segments()
        .and_then(|mut s| s.next())
        .map(|s| s.to_string());
    let username = if u.username().is_empty() {
        None
    } else {
        Some(u.username().to_string())
    };
    let password = u.password().map(|p| p.to_string());
    let mut redacted = u;
    let _ = redacted.set_password(None);
    Some(ParsedUri {
        uri: redacted.to_string(),
        host: Some(host),
        port,
        database,
        username,
        password,
    })
}

/// 连接时把 Keychain 密码补回 URI(URI 若已带密码则保留原样)。
pub(crate) fn inject_password_into_uri(uri: &str, password: Option<&str>) -> String {
    let Ok(mut u) = url::Url::parse(uri) else {
        return uri.to_string();
    };
    if u.password().is_none()
        && let Some(p) = password
        && !p.is_empty()
    {
        let _ = u.set_password(Some(p));
    }
    u.to_string()
}

pub(crate) async fn test_connection(
    source: DataSource,
    password: Option<String>,
) -> Result<(), String> {
    match source.driver {
        DriverKind::Postgres | DriverKind::MySQL | DriverKind::Sqlite => {
            let url = build_sql_url(&source, password.as_deref());
            let pool = sqlx::AnyPool::connect(&url)
                .await
                .map_err(|e| e.to_string())?;
            sqlx::query("SELECT 1")
                .execute(&pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        DriverKind::MongoDB => {
            let url = build_mongo_url(&source, password.as_deref());
            let client = mongodb::Client::with_uri_str(&url)
                .await
                .map_err(|e| e.to_string())?;
            client
                .list_database_names()
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}
