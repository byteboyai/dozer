use crate::AgentKind;
use serde::{Deserialize, Serialize};

/// `SetTodoStatus` 期望的**已存储**逻辑状态。注意"进行中"(派发到某个
/// 存活会话)不是纯存储态——它由 `dispatch_session_id` 的存活与否在服务端
/// 之外(GUI)推导,不在此列。三值各自对应 `todos` 表 `done`/`paused`/
/// `dispatch_*` 的组合,服务端一次 UPDATE 原子落定:
/// - `Todo`(待办):撤销完成、撤销搁置、摘掉派发记录 → 变为可再排队的待办。
/// - `Suspended`(搁置):撤销完成、置搁置、同时摘掉派发记录(搁置 = 拿回,
///   不残留"还在某个会话手上")。
/// - `Done`(已完成):置完成、撤销搁置(与勾选 `ToggleTodo(true)` 等价)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStoredStatus {
    Todo,
    Suspended,
    Done,
}

/// 一条任务(`dozerd` 的 `todos` 表一行)。`id` 是稳定身份,取代 v1 时
/// 靠文本哈希关联元数据的做法(2026-09-01 SQLite 迁移)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoInfo {
    pub id: i64,
    pub project_id: i64,
    pub text: String,
    pub done: bool,
    /// 搁置(`1` = 已暂停不推进)。独立于 `done`,是第四种派生状态"搁置"
    /// (`!done && paused`)的存储来源;`done` 真时以"已完成"优先。记录一次
    /// 任务被"拿回/停用"的历史,让任务语义更接近现实的"暂停后待重启"。
    #[serde(default)]
    pub paused: bool,
    /// 排序键:同一 project 下,展示查询固定 `ORDER BY done, paused, rank`
    /// ——进行中/待办按 rank 升序在最前、搁置其次、已完成沉底;同一分组内
    /// 比较才有意义,跨分组数值不保证可比。
    pub rank: i64,
    pub created_ms: u64,
    pub completed_at_ms: Option<u64>,
    /// "MM-DD" 格式,GUI 日历选择器写入。
    pub plan_date: Option<String>,
    pub dispatch_session_id: Option<String>,
    pub dispatch_at_ms: Option<u64>,
    /// 所属分类节点 id,`None` = 未分类(2026-09-01 分类树设计新增)。
    #[serde(default)]
    pub category_id: Option<i64>,
    /// 指派给哪个 agent 种类(纯记录,不触发执行;`None` = 未指派)。只有
    /// `AgentKind::label()` 有 headless 适配器的四种(Claude/Codebuddy/
    /// Opencode/V8agent)会出现在这里,选择器不出现其余三种。
    #[serde(default)]
    pub assigned_agent: Option<AgentKind>,
}

/// 一个分类树节点(`dozerd` 的 `todo_categories` 表一行)。`parent_id ==
/// None` 表示顶层节点。作用域按 `project_id` 隔离,不跨项目共享
/// (2026-09-01 分类树设计)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategoryInfo {
    pub id: i64,
    pub project_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    /// 同一 `parent_id` 下的兄弟排序键,升序展示。
    pub rank: i64,
    pub created_ms: u64,
    /// 该分类下的任务是否开启自动轮询处理(dozerd 后台定时扫描触发
    /// headless 处理),默认关闭。
    #[serde(default)]
    pub auto_poll_enabled: bool,
}

/// `MoveCategorySibling` 的方向:与同一 `parent_id` 下相邻的前一个/
/// 后一个兄弟节点交换 `rank`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryMoveDirection {
    Up,
    Down,
}
