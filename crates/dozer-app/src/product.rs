//! 产品组合根(bytehost H7):Dozer 的面板清单——有哪些面板、标题与图标、默认栏位与顺序、
//! 旧版落盘布局的迁移。host 只通过 `panel_registry` 认清单,不知道这里的具体面板;换一个产品
//! (如 Digger)就是另写一份这样的函数。

use byteui::interaction::icons::IconKind;

use crate::app::PanelKind;
use crate::chrome::rail::migrate_legacy_rail;
use crate::panel_registry::{PanelCatalog, PanelDescriptor};

pub(crate) fn dozer_catalog() -> PanelCatalog {
    use PanelKind::*;
    // 面板标题走 i18n key + 英文回退(见 `PanelDescriptor` 与 `crate::i18n`);
    // 中文译文在 `locales/host/zh-CN.json` 的 `panel.*`,这里不再内嵌中文。
    let d = |id, icon, title_key, title_fallback| PanelDescriptor {
        id,
        title_key,
        title_fallback,
        icon,
    };
    PanelCatalog::new(
        vec![
            d(Files, IconKind::FolderTree, "panel.files", "Files"),
            d(GitLog, IconKind::GitGraph, "panel.git_log", "Git Log"),
            d(Todo, IconKind::ListTodo, "panel.todo", "Todo"),
            d(Project, IconKind::Briefcase, "panel.project", "Project"),
            d(Database, IconKind::Database, "panel.database", "Database"),
            d(Ssh, IconKind::Server, "panel.ssh", "SSH Hosts"),
            d(Web, IconKind::Globe, "panel.web", "Browser"),
            d(Agent, IconKind::Brain, "panel.agent", "Agent"),
            d(
                GroupChat,
                IconKind::SquareSparkles,
                "panel.group_chat",
                "Group Chat",
            ),
            d(
                Conversations,
                IconKind::BotMessageSquare,
                "panel.conversations",
                "Conversations",
            ),
            d(Usage, IconKind::BarChart3, "panel.usage", "Usage"),
            d(
                CodeHealth,
                IconKind::SquareActivity,
                "panel.code_health",
                "Code Health",
            ),
        ],
        vec![Project, Todo, Files, GitLog, Database, Ssh, Web],
        vec![Agent, GroupChat, Conversations, Usage, CodeHealth],
        migrate_legacy_rail,
    )
    .expect("Dozer 面板清单自检失败")
}
