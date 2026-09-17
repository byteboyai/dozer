//! Database 面板状态类型:DriverKind/DataSource/TableRef/SchemaState/Draft/
//! WorkspaceState/AppState/Message。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use super::*;

/// 数据库面板支持的驱动类型。穷举枚举,不做插件机制(见设计文档"非目标")。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DriverKind {
    #[default]
    Postgres,
    MySQL,
    Sqlite,
    MongoDB,
}

impl DriverKind {
    pub const ALL: [DriverKind; 4] = [
        DriverKind::Postgres,
        DriverKind::MySQL,
        DriverKind::Sqlite,
        DriverKind::MongoDB,
    ];

    /// 面板/表单里展示的中文名。
    pub fn label(self) -> &'static str {
        match self {
            DriverKind::Postgres => "PostgreSQL",
            DriverKind::MySQL => "MySQL",
            DriverKind::Sqlite => "SQLite",
            DriverKind::MongoDB => "MongoDB",
        }
    }
}

/// 新增/编辑数据源表单里"数据源类型"select 的一个选项:驱动 + 预先算好的
/// 展示文案(可能带"(已禁用)"后缀,见 `source_form`)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DriverOption {
    pub(crate) driver: DriverKind,
    pub(crate) label: String,
}

impl std::fmt::Display for DriverOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label)
    }
}

/// 一个数据源(不含密码)。`.dozer/database.json` 存 `Vec<DataSource>`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataSource {
    pub id: String,
    pub name: String,
    pub driver: DriverKind,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// SQLite 这里存文件路径,其它驱动存库名。
    pub database: Option<String>,
    pub username: Option<String>,
    /// 可选连接 URI(Postgres/MySQL,设计文档"可整串粘贴")。持久化时密码会被
    /// 抽取到 Keychain、URI 里不留明文;连接时若缺密码再从 Keychain 补回。
    pub uri: Option<String>,
}

/// 某条数据源当前的连接测试状态,画在卡片上。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TestStatus {
    #[default]
    Idle,
    Testing,
    Ok,
    Err(String),
}

/// schema 树里的一个表/视图(阶段 2)。
#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    /// 仅 Postgres 有值(table_schema);MySQL/SQLite 恒 None。
    pub schema: Option<String>,
    pub name: String,
    pub is_view: bool,
}

/// 列元信息(阶段 2)。主键/索引/默认值不在本阶段范围(设计文档"非目标")。
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
}

/// 某张表的列加载进度(阶段 2)。
#[derive(Debug, Clone)]
pub enum ColumnLoad {
    Loading,
    Loaded(Vec<ColumnInfo>),
    Failed(String),
}

/// 单个数据源的 schema 树状态(**纯内存,不持久化**——schema 是即时快照,
/// 重启后重新拉,展开状态不值得落盘)。
#[derive(Debug, Default)]
pub struct SchemaState {
    pub(crate) loading_tables: bool,
    pub(crate) tables_error: Option<String>,
    /// 按 (schema, name) 有序(SQL 层 ORDER BY,客户端不再排序)。
    pub(crate) tables: Vec<TableRef>,
    /// Postgres schema 节点展开集。
    pub(crate) expanded_schemas: HashSet<String>,
    /// 表节点展开集,key = (schema, name)。
    pub(crate) expanded_tables: HashSet<(Option<String>, String)>,
    /// 每表列缓存,key = (schema, name)。
    pub(crate) columns: HashMap<(Option<String>, String), ColumnLoad>,
}

impl SchemaState {
    pub fn loading_tables(&self) -> bool {
        self.loading_tables
    }

    pub fn tables_error(&self) -> Option<&str> {
        self.tables_error.as_deref()
    }

    pub fn tables(&self) -> &[TableRef] {
        &self.tables
    }

    /// 指定表的列加载态(单测断言与过期防线用)。
    #[cfg(test)]
    pub fn column_load(&self, key: &(Option<String>, String)) -> Option<&ColumnLoad> {
        self.columns.get(key)
    }
}

/// schema 树可见行(摊平结果)。
pub struct SchemaRow<'a> {
    pub kind: SchemaRowKind<'a>,
    pub depth: usize,
    pub expanded: bool,
}

/// schema 树行类型。`ColumnsLoading`/`ColumnsFailed` 是展开表之后的占位行。
pub enum SchemaRowKind<'a> {
    Schema(&'a str),
    Table(&'a TableRef),
    Column(&'a ColumnInfo),
    ColumnsLoading,
    ColumnsFailed(&'a str),
}

/// 把 `SchemaState` 摊平成可见行(纯函数,单测友好;渲染侧单层循环)。
/// Postgres 比 MySQL/SQLite 多一层 schema 节点(schema 节点不做异步加载,
/// 表列表一次查全后客户端按 `table_schema` 分组)。
pub fn tree_rows(state: &SchemaState, driver: DriverKind) -> Vec<SchemaRow<'_>> {
    let mut out: Vec<SchemaRow<'_>> = Vec::new();
    if driver == DriverKind::Postgres {
        let mut by_schema: BTreeMap<&str, Vec<&TableRef>> = BTreeMap::new();
        for t in &state.tables {
            by_schema
                .entry(t.schema.as_deref().unwrap_or("public"))
                .or_default()
                .push(t);
        }
        for (schema, tables) in by_schema {
            let expanded = state.expanded_schemas.contains(schema);
            out.push(SchemaRow {
                kind: SchemaRowKind::Schema(schema),
                depth: 0,
                expanded,
            });
            if expanded {
                push_table_rows(&mut out, state, &tables, 1);
            }
        }
    } else {
        let tables: Vec<&TableRef> = state.tables.iter().collect();
        push_table_rows(&mut out, state, &tables, 0);
    }
    out
}

fn push_table_rows<'a>(
    out: &mut Vec<SchemaRow<'a>>,
    state: &'a SchemaState,
    tables: &[&'a TableRef],
    depth: usize,
) {
    for t in tables {
        let key = (t.schema.clone(), t.name.clone());
        let expanded = state.expanded_tables.contains(&key);
        out.push(SchemaRow {
            kind: SchemaRowKind::Table(t),
            depth,
            expanded,
        });
        if !expanded {
            continue;
        }
        match state.columns.get(&key) {
            // 展开动作总会伴随加载触发,正常到不了这里;防御性忽略。
            None => {}
            Some(ColumnLoad::Loading) => out.push(SchemaRow {
                kind: SchemaRowKind::ColumnsLoading,
                depth: depth + 1,
                expanded: false,
            }),
            Some(ColumnLoad::Failed(e)) => out.push(SchemaRow {
                kind: SchemaRowKind::ColumnsFailed(e),
                depth: depth + 1,
                expanded: false,
            }),
            Some(ColumnLoad::Loaded(cols)) => {
                for c in cols {
                    out.push(SchemaRow {
                        kind: SchemaRowKind::Column(c),
                        depth: depth + 1,
                        expanded: false,
                    });
                }
            }
        }
    }
}

/// 挂在 `App` 上:哪些驱动类型在"新增数据源"下拉里可选。默认全部启用。
#[derive(Debug)]
pub struct AppState {
    pub(crate) enabled: std::collections::HashSet<DriverKind>,
    /// 驱动管理弹层的开关态(非持久化 UI 态,不参与 `save()`)。
    pub(crate) drivers_popup_open: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            enabled: DriverKind::ALL.into_iter().collect(),
            drivers_popup_open: false,
        }
    }
}

impl AppState {
    pub fn load() -> Self {
        let path = drivers_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(file) = serde_json::from_str::<EnabledDriversFile>(&text) else {
            return Self::default();
        };
        Self {
            enabled: file.enabled.into_iter().collect(),
            drivers_popup_open: false,
        }
    }

    fn save(&self) {
        let file = EnabledDriversFile {
            enabled: self.enabled.iter().copied().collect(),
        };
        let Ok(json) = serde_json::to_string_pretty(&file) else {
            return;
        };
        if let Err(e) = std::fs::write(drivers_path(), json) {
            tracing::warn!("写入 database_drivers.json 失败: {e}");
        }
    }

    pub fn is_enabled(&self, driver: DriverKind) -> bool {
        self.enabled.contains(&driver)
    }

    pub fn toggle(&mut self, driver: DriverKind) {
        if !self.enabled.remove(&driver) {
            self.enabled.insert(driver);
        }
        self.save();
    }

    /// 驱动管理弹层的开关态(非持久化 UI 态)。
    pub fn drivers_popup_open(&self) -> bool {
        self.drivers_popup_open
    }
}

/// 新增/编辑数据源表单的草稿态。`id` 为 `None` = 新增,`Some(..)` = 编辑
/// 已有数据源(保留原 id 不变)。
#[derive(Debug, Clone, Default)]
pub struct DataSourceDraft {
    pub id: Option<String>,
    pub name: String,
    pub driver: DriverKind,
    pub host: String,
    pub port: String,
    pub database: String,
    pub username: String,
    pub password: String,
    /// 可选连接 URI 输入(Postgres/MySQL)。保存时优先用它,并抽取密码进 Keychain。
    pub uri: String,
}

/// 挂在每个 `Workspace` 上:当前项目配置的数据源列表 + 编辑态 + 每条数据源
/// 的连接测试状态 + schema 树浏览态(阶段 2,纯内存)。
#[derive(Default)]
pub struct WorkspaceState {
    pub(crate) sources: Vec<DataSource>,
    pub(crate) editing: Option<DataSourceDraft>,
    pub(crate) test_status: HashMap<String, TestStatus>,
    /// 数据源树里当前展开(显示 schema/表)的数据源 id 集合——允许多个
    /// 根节点同时展开,不再是单选的"进入/返回"整页切换(设计文档回顾里
    /// 的卡片列表已改成内联树,见 `view`)。
    pub(crate) expanded_sources: HashSet<String>,
    /// 每个数据源 id 一份 schema 树状态(阶段 2,纯内存)。
    pub(crate) schemas: HashMap<String, SchemaState>,
    /// 右侧内容窗格状态(表/集合/查询 tab)。纯内存,不持久化——同
    /// `schemas`(阶段 2),重启后 tab 全部关闭,不留痕迹。
    pub(crate) content: DatabaseContentState,
    /// 正在等用户确认删除的数据源 id。`Some` 时面板顶部覆盖一层确认
    /// 对话框(同 `ssh::WorkspaceState::delete_confirm` 的既有设计:右键
    /// 菜单"删除"只记待确认态,真正删除要等确认框里点确认才触发
    /// `DeleteSource`)——2026-09 补上,此前"删除"在菜单里点一下就直接
    /// 删,跟主机/项目/文件树都有二次确认不一致。
    pub(crate) delete_confirm: Option<String>,
    /// 新增/编辑表单**任意一个字段**是否持有 iced 真实焦点——main.rs 每帧用
    /// `CaptureFormFocus`/`take_form_focused` 查回来写进这里(同 Files 搜索框
    /// `search_focused` 的既有手法)。`App::database_form_open` 键盘路由用它
    /// 判断要不要放行给标准 iced 管线,不再只看"表单是否打开"(2026-09 用户
    /// 反馈:数据库面板与 Agent 终端分栏同屏时,表单开着但用户点进的是终端
    /// 输入框,旧信号仍卡真导致终端打不进字)。
    pub(crate) form_focused: bool,
    /// 表单里"测试连接"按钮(`DraftTestConnection`)的测试状态——**不**复用
    /// `test_status`(那张表按"已保存数据源的 id"记账):新增数据源还没有
    /// id,编辑数据源时表单里的值也可能跟已保存的不一样(用户正在改还没点
    /// 保存),用同一张表会导致测试结果要么记不进去、要么把"草稿"的测试
    /// 结果误写成"已保存数据源"的连接状态,污染数据源树上显示的那份。
    /// 表单打开/关闭(`AddSourceStart`/`EditSourceStart`/`DraftCancel`/
    /// `DraftSave`)时重置,不跨表单会话保留。
    pub(crate) draft_test_status: TestStatus,
}

impl std::fmt::Debug for WorkspaceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceState")
            .field("sources", &self.sources)
            .field("editing", &self.editing)
            .field("test_status", &self.test_status)
            .field("draft_test_status", &self.draft_test_status)
            .field("expanded_sources", &self.expanded_sources)
            .field("schemas", &self.schemas)
            .field("content_tab_count", &self.content.tabs().len())
            .finish()
    }
}

impl WorkspaceState {
    pub fn sources(&self) -> &[DataSource] {
        &self.sources
    }

    pub fn editing(&self) -> Option<&DataSourceDraft> {
        self.editing.as_ref()
    }

    /// 新增/编辑表单任意字段是否持有真实焦点(main.rs 键盘路由用)。
    pub fn form_focused(&self) -> bool {
        self.form_focused
    }

    /// 每帧渲染循环用 `take_form_focused` 查回来的真实焦点态写进这里。
    pub fn set_form_focused(&mut self, focused: bool) {
        self.form_focused = focused;
    }

    pub fn test_status(&self, source_id: &str) -> &TestStatus {
        self.test_status.get(source_id).unwrap_or(&TestStatus::Idle)
    }

    /// 表单里"测试连接"按钮的测试状态,见 `draft_test_status` 字段文档。
    pub fn draft_test_status(&self) -> &TestStatus {
        &self.draft_test_status
    }

    /// 该数据源在树里是否已展开(header 行 chevron 状态 + 右键菜单"刷新"
    /// 是否可用都靠它判断)。
    pub fn is_expanded(&self, source_id: &str) -> bool {
        self.expanded_sources.contains(source_id)
    }

    /// 已展开数据源的 schema 树状态只读视图(`None` = 尚未加载/已被删除,
    /// 调用方按"加载中"渲染即可,不专门清理,同设计文档 §2 过期防线)。
    pub fn schema_state(&self, source_id: &str) -> Option<&SchemaState> {
        self.schemas.get(source_id)
    }

    /// 右侧内容窗格状态只读视图(视图层 Task 7/8 消费)。
    pub fn content(&self) -> &DatabaseContentState {
        &self.content
    }

    /// 右侧内容窗格状态的可变视图。仅供 `app.rs` 拦截 `TabOverflowToggle`/
    /// `TabOverflowDismiss`(这两个消息需要 `App::last_cursor`,不进
    /// `database::update`)时对溢出锚点做切换时使用。
    pub(crate) fn content_mut(&mut self) -> &mut DatabaseContentState {
        &mut self.content
    }

    /// 正在等确认删除的数据源 id(`None` = 没有)。给 `view()` 判是否覆盖
    /// 确认对话框。
    pub fn delete_confirm(&self) -> Option<&str> {
        self.delete_confirm.as_deref()
    }
    /// 记"用户点了删除,想删这个数据源"——只记待确认态,真正删除要等
    /// 确认框里的确认按钮(走 `Message::DeleteSource`)。
    pub(crate) fn request_delete(&mut self, source_id: String) {
        self.delete_confirm = Some(source_id);
    }
    /// 取消删除确认(点对话框外的遮罩/取消按钮),清掉待确认态。
    pub(crate) fn cancel_delete(&mut self) {
        self.delete_confirm = None;
    }
}

/// 内容窗格 tab 栏里某个可悬停部件的身份;配合 `Message::TabHover` 由内核
/// 转发到 `HoverId::DatabaseTabItem/DatabaseTabClose`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseTabHoverTarget {
    Title,
    Close,
}

/// 连接测试 / 表单交互的统一消息。`TestConnectionResult` 特化携带
/// `project_id`——异步结果可能晚于用户切换项目才回来,必须按这个项目 id
/// 而不是"当前聚焦项目"路由回正确的 `WorkspaceState`。
#[derive(Debug, Clone)]
pub enum Message {
    /// 驱动管理弹层:勾/取消勾某个驱动类型。
    ToggleDriver(DriverKind),
    /// 打开"驱动管理"弹层。
    DriversPopupToggle,
    /// 点"＋新增数据源"→ 打开空白草稿表单。
    AddSourceStart,
    /// 点某张卡的"编辑"→ 用该数据源现有字段(不含密码)预填草稿表单。
    EditSourceStart(String),
    /// 表单字段编辑(草稿态,未提交)。
    DraftNameChanged(String),
    DraftDriverChanged(DriverKind),
    DraftHostChanged(String),
    DraftPortChanged(String),
    DraftDatabaseChanged(String),
    DraftUsernameChanged(String),
    DraftPasswordChanged(String),
    /// 表单里的"连接 URI"输入变更(Postgres/MySQL 便捷粘贴)。
    DraftUriChanged(String),
    /// 任意数据源输入框/SQL 编辑器被右键:内核拦截,不进 `update`——转发成
    /// 顶层 `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
    /// 内容侧"收起/展开列表列"按钮:内核拦截,不进 `update`——转发成顶层
    /// `Message::TogglePanelListCollapse(PanelKind::Database)`(见 app.rs)。
    ToggleListCollapse,
    /// 提交表单:新增或更新(视 `draft.id` 是否为 `None`),写盘 + 密码进
    /// Keychain,关闭表单。
    DraftSave,
    /// 取消表单,丢弃草稿。
    DraftCancel,
    /// 表单里的"测试连接"按钮:直接拿当前草稿里的字段值去连,不要求先
    /// 保存——跟 `TestConnection` 的区别是后者只能测已保存的数据源(按
    /// id 查 `ws_state.sources`),新增数据源这时候还没有 id,编辑数据源
    /// 时表单里也可能是用户改了但还没点保存的值。结果写
    /// `ws_state.draft_test_status`,不进 `test_status` 那张表(见该字段
    /// 文档)。
    DraftTestConnection,
    /// 表单"测试连接"异步结果。不带 `project_id`——跟 `TestConnectionResult`
    /// 不同,这里不严格要求跨项目路由:用户即使中途取消表单、切到别的
    /// 项目,迟到的结果最多写进当前聚焦项目"看不见"的 `draft_test_status`
    /// 字段(只有表单开着才会渲染),不会串到别的数据源卡片上;真正的防线
    /// 是 `AddSourceStart`/`EditSourceStart` 每次打开表单都重置这个字段,
    /// 不会把上一次(甚至上一个项目)的残留结果带出来。
    DraftTestConnectionResult(Result<(), String>),
    /// 右键菜单点"删除":只记待确认态,弹出确认对话框,不立即删
    /// (同 `ssh::Message::DeleteHostRequest`)。
    DeleteSourceRequest(String),
    /// 确认框里的"取消"/点遮罩:清掉待确认态,不删任何东西。
    DeleteSourceCancel,
    /// 确认框里的"删除"才真正执行:同时删 `.dozer/database.json` 里的
    /// 记录和 Keychain 里的密码条目。
    DeleteSource(String),
    /// 点"测试连接":发起异步测试,`ws_state.test_status` 先置
    /// `Testing`。
    TestConnection(String),
    /// 异步测试结果。**带 `project_id`**——见设计文档"结果经 `emit`
    /// 回传"一节,不能只带 `source_id`,用户可能在等待期间切走了项目
    /// 页签,`App::update` 必须按这里的 `project_id` 而不是"当前聚焦
    /// 项目"路由。
    TestConnectionResult(i64, String, Result<(), String>),
    /// 数据源树 header 行左键点(chevron/名字):展开/收起该源的 schema 树。
    /// 首次展开且从未加载过才自动发起表加载(有缓存/有错误保留现状,
    /// 错误态由右键菜单"刷新"触发)。允许多个源同时展开。
    ToggleSourceExpanded(String),
    /// 数据源树 header 行右键:内核拦截,不进 `update`——转发成顶层
    /// `App::database_source_context_menu` 弹出测试连接/编辑/删除/刷新
    /// 菜单(见 app.rs)。
    SourceContextMenu(String),
    /// 右键菜单"刷新":重拉表列表(旧快照保留不闪空,成功后对账)。处理前
    /// 先核对该源当前确实展开着,防菜单残留动作作用到已收起的源。
    SchemaRefresh(String),
    /// 任意顶部 `HoverId` 的悬停进入/离开(内容侧收起按钮等)。内核拦截转发
    /// 给顶层 `App::set_hover`,本面板 `update` 保 no-op 分支维持 match 穷尽。
    Hover(crate::app::HoverId, bool),
    /// 内容窗格 tab 栏某个 tab 的悬停进入/离开;纯转发动机,`update()` 里
    /// 保 no-op 分支维持 match 穷尽,真正接线在 `app.rs` 的特化臂。
    TabHover(DatabaseTabHoverTarget, usize, bool),
    /// Postgres schema 节点展开/收起(纯同步,不触发加载)。带 `source_id`——
    /// 允许多个源同时展开后,不能再靠单一"当前浏览源"隐式定位。
    ToggleSchema(String, String),
    /// 表节点展开/收起;展开时列缓存缺失或曾失败 → 置 `Loading` 并发起列加载。
    ToggleTable {
        source_id: String,
        schema: Option<String>,
        table: String,
    },
    /// 表清单异步结果。**带 `project_id`**——结果可能晚于用户切走项目页签,
    /// 必须按自带 id 路由(同 `TestConnectionResult` 的口径)。
    TablesLoaded(i64, String, Result<Vec<TableRef>, String>),
    /// 列加载异步结果,带 `project_id`/`source_id` 双路由。
    ColumnsLoaded {
        project_id: i64,
        source_id: String,
        schema: Option<String>,
        table: String,
        result: Result<Vec<ColumnInfo>, String>,
    },

    // ---- 右侧内容窗格:表/集合/查询 tab 的开关 ----
    /// schema 树点一张表/视图的行内文字 → 开(或聚焦已开的)浏览 tab。
    OpenTableTab {
        source_id: String,
        schema: Option<String>,
        table: String,
    },
    /// schema 树点一个 MongoDB 集合 → 开(或聚焦已开的)浏览 tab。
    OpenCollectionTab {
        source_id: String,
        name: String,
    },
    /// schema 树头部"+ 新查询" → 永远新开一个查询 tab。
    OpenQueryTab(String),
    /// tab 栏点某个 tab → 切换 active(索引)。
    SelectTab(usize),
    /// tab 栏点最前面那个固定的"空白"占位 tab(不对应 `tabs` 里任何一条
    /// 记录,`content.active_idx() == None` 即代表它处于选中态,参考
    /// `extensions/ssh.rs::Message::SelectBlankTab` 同款设计)。
    SelectBlankTab,
    /// tab 栏点 × → 关闭(索引)。
    CloseTab(usize),
    /// tab 栏溢出下拉开关,语义同顶层 `Message::TermTabOverflowToggle`。由
    /// `App::update` 拦截处理(需要 `App::last_cursor`),不进 `database::update`。
    TabOverflowToggle,
    /// tab 栏溢出下拉:点击外部关闭。同样由 `App::update` 拦截。
    TabOverflowDismiss,

    // ---- 浏览页(WHERE/ORDER BY/分页) ----
    BrowseWhereChanged(usize, String),
    BrowseOrderByChanged(usize, String),
    BrowsePageSizeChanged(usize, u32),
    BrowsePrev(usize),
    BrowseNext(usize),
    /// 显式"运行"(WHERE 框回车/翻页/改页大小/改排序 统一走这个,见 Task 7)。
    BrowseRun(usize),
    /// 异步结果,带 `project_id` + tab 稳定 id(路由口径同
    /// `TablesLoaded`/`ColumnsLoaded`)。
    /// 第三个字段是发起这轮请求时 `BrowseState::begin_run()` 返回的
    /// `run_seq`——落地前必须核对它仍是当前值(`BrowseState::is_current_run`),
    /// 只查 `loading` 布尔标志分辨不出"哪一轮"结果,连续两次改 WHERE 会让
    /// 旧结果覆盖新结果(设计文档"架构与数据流 §7"的过期防线,数字比对是
    /// 必须的,不能简化成布尔)。
    BrowseResult(i64, usize, u64, Result<BrowsePage, String>),

    // ---- SQL 查询控制台 ----
    /// 编辑器动作(`text_editor::Action`),`update()` 只管
    /// `content.sql.perform(action)`,同 `todo.rs::AddEdit` 的既有用法。
    QueryTextAction(usize, iced_widget::text_editor::Action),
    QueryRun(usize),
    /// 第三个字段同 `BrowseResult`,是 `QueryState::begin_run()` 的 `run_seq`。
    QueryResult(i64, usize, u64, Result<QueryOutcome, String>),
}
