//! 产品组合根(bytehost H7):Dozer 的面板清单——有哪些面板、标题与图标、默认栏位与顺序、
//! 旧版落盘布局的迁移。host 只通过 `panel_registry` 认清单,不知道这里的具体面板;换一个产品
//! (如 Digger)就是另写一份这样的函数。

use byteui::interaction::icons::IconKind;

use crate::app::PanelKind;
use crate::chrome::rail::migrate_legacy_rail;
use crate::panel_registry::{PanelCatalog, PanelDescriptor};

pub(crate) fn dozer_catalog() -> PanelCatalog {
    use PanelKind::*;
    let d = |id, icon, title| PanelDescriptor { id, title, icon };
    PanelCatalog::new(
        vec![
            d(Files, IconKind::FolderTree, "文件"),
            d(GitLog, IconKind::GitGraph, "Git Log"),
            d(Todo, IconKind::ListTodo, "待办"),
            d(Project, IconKind::Briefcase, "项目"),
            d(Database, IconKind::Database, "数据库"),
            d(Ssh, IconKind::Server, "SSH 主机"),
            d(Web, IconKind::Globe, "浏览器"),
            d(Agent, IconKind::Brain, "代理"),
            d(GroupChat, IconKind::SquareSparkles, "群聊"),
            d(Conversations, IconKind::BotMessageSquare, "对话"),
            d(Usage, IconKind::BarChart3, "用量"),
            d(CodeHealth, IconKind::SquareActivity, "代码健康度"),
        ],
        vec![Project, Todo, Files, GitLog, Database, Ssh, Web],
        vec![Agent, GroupChat, Conversations, Usage, CodeHealth],
        migrate_legacy_rail,
    )
    .expect("Dozer 面板清单自检失败")
}
