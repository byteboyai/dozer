//! 面板库与产品之间的接缝:库只定义窄 trait,产品注入实现。
//!
//! 这样库的依赖闭包不含 dozer-app/dozer-client/dozer-core(计划 §Task 19 第 5 条):
//! - Git 统计不再是库内直连 `bytegit::Repo`,改为产品注入 [`GitStatsProvider`]
//!   (计划第 3 条:"直接 bytegit Repo 读取改为面板所需的窄数据提供接口")。
//! - 一次性提示不再用产品的 `toast::Outbox`(带 `dozer_core::log::Scope`),
//!   改为库自己的 [`UsageOutbox`] + [`UsageNotice`];产品排空后转成自己的 Toast。
//! - 推送到 webview 的脚本构造(产品的 `dispatch_script`)改为注入 [`ScriptDispatch`]。
//! - 展示语义(agent 图标/颜色、面板头、列表折叠按钮等)由产品的 [`PanelDisplay`] 提供。

use dozer_model::AgentKind;
use iced_widget::core::{Color, Element};
use std::collections::BTreeMap;
use std::path::Path;

/// 面板需要的 Git 统计窄接口。Dozer 实现包 `bytegit::Repo`;Digger 提供自己的
/// 语义(或显式 Unavailable)。库不关心 Git 实现。
pub trait GitStatsProvider: Send + Sync {
    /// 项目仓库 HEAD 可达的提交总数;非 git 仓库/任何错误记 0。
    fn commit_count(&self, project_path: &Path) -> u64;
    /// HEAD 可达提交按 UTC 提交日的逐日计数;非 git 仓库/任何错误返回空。
    fn commits_by_day(&self, project_path: &Path) -> BTreeMap<i64, u64>;
}

/// 不提供 Git 统计的实现(如 Digger 尚未接 Git):恒 0 / 空。可用 `Default`。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoGitStats;

impl GitStatsProvider for NoGitStats {
    fn commit_count(&self, _project_path: &Path) -> u64 {
        0
    }
    fn commits_by_day(&self, _project_path: &Path) -> BTreeMap<i64, u64> {
        BTreeMap::new()
    }
}

/// 库内一次性提示的级别(与产品的 Toast 级别同形,但不依赖产品的 `Level`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// 一条待发提示。产品排空后按自己的方式呈现(如转成 `toast::Pending` 并写入
/// 自己的日志来源)。
#[derive(Debug, Clone, PartialEq)]
pub struct UsageNotice {
    pub level: NoticeLevel,
    pub text: String,
    /// 去重键:同 key 再推 = 刷新文本/级别,不新增。
    pub key: Option<String>,
}

/// 库内待发提示队列。与产品的 `toast::Outbox` 同模式(`push` 后由产品排空),
/// 但只依赖本库类型,不引入 `dozer_core`。
#[derive(Debug, Default)]
pub struct UsageOutbox {
    items: Vec<UsageNotice>,
}

impl UsageOutbox {
    pub fn push(&mut self, level: NoticeLevel, text: impl Into<String>, key: impl Into<String>) {
        self.items.push(UsageNotice {
            level,
            text: text.into(),
            key: Some(key.into()),
        });
    }

    pub fn push_plain(&mut self, level: NoticeLevel, text: impl Into<String>) {
        self.items.push(UsageNotice {
            level,
            text: text.into(),
            key: None,
        });
    }

    pub fn take(&mut self) -> Vec<UsageNotice> {
        std::mem::take(&mut self.items)
    }
}

/// 把一个已序列化的 payload 包成 `evaluate_script` 脚本字符串。Dozer 注入
/// `preview::dispatch_script`;其他产品注入自己的等价实现。返回 `String` 以便
/// `surface.rs` 在纯函数里构造(实际下发仍由产品完成)。
pub type ScriptDispatch = fn(&str) -> String;

/// 产品展示适配:面板 view 需要的产品专属语义(agent 图标/颜色、面板头、
/// 列表折叠按钮、悬停事件映射)。泛型 `M` 是产品的面板 `Message` 类型。
///
/// 计划第 2 条:"将 workspace/chrome UI helper 依赖替换为公共 UI 或注入的产品
/// 展示适配"。这里走"注入的产品展示适配":通用组件(byteui)已在库里直接用,
/// 只有产品语义(图标表、面板头样式、折叠按钮交互)经本 trait 注入。
pub trait PanelDisplay<M> {
    /// 把库内的面板 [`crate::state::Message`] 包成产品的顶层 `Message`(通常是
    /// 产品 `Message` 里 wraps usage `Message` 的那个变体)。
    fn wrap(&self, message: crate::state::Message) -> M;
    /// agent 品牌图标色(同产品别处圆点配色)。仅"各 agent"项用;"全部agent"
    /// 项用公共 `BarChart3` 图标 + 选中态色,不经此接口。
    fn agent_icon_color(&self, agent: AgentKind) -> Color;
    /// agent 品牌图标元素,已按 `color` 着色(品牌图自带原色时忽略 `color`)。
    /// 库内 `agent_filter_button` 的图标槽。
    fn agent_icon(
        &self,
        agent: AgentKind,
        color: Color,
    ) -> Element<'static, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 面板头(图标 + 标题 + 分割线,右侧可选操作元素)。
    fn panel_head<'a>(
        &self,
        title: &'a str,
        actions: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 面板列表列的折叠/展开按钮。
    fn list_collapse_button<'a>(
        &self,
        collapsed: bool,
        tooltip_collapse: impl Into<std::borrow::Cow<'a, str>>,
        tooltip_expand: impl Into<std::borrow::Cow<'a, str>>,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 折叠按钮悬停事件(进入/离开)。产品把它映射成自己的顶层悬停键。
    fn collapse_hover(&self, hovered: bool) -> M;
    /// 折叠按钮点击消息("收起/展开列表列")。
    fn toggle_list_collapse(&self) -> M;
}
