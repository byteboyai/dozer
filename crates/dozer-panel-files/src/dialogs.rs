//! 面板自带的确认对话框内容。只产出 `byteui` 的 `ConfirmDialog` 数据,
//! 由产品放进取景树。

use byteui::interaction::icons;
use crate::i18n::{t, t_args};
use crate::state::WorkspaceState;
use crate::Message;

pub fn delete_confirm_spec(
    ws_state: &WorkspaceState,
    locale: &str,
) -> Option<byteui::feedback::dialog::ConfirmDialog<Message>> {
    let targets = ws_state.tree_delete_confirm.as_ref()?;
    if targets.is_empty() {
        return None;
    }
    // 各待删除项的相对项目根路径(同 `files_move_card` 的 source_display
    // 手法),比单文件名更能定位目标——同名文件散在不同子目录时不再有歧义;
    // 文件树尚未加载/路径不在根下时兜底完整路径原文。
    let rel_paths: Vec<String> = targets
        .iter()
        .map(|(p, _)| {
            ws_state
                .file_tree
                .as_ref()
                .and_then(|t| p.strip_prefix(t.root()).ok())
                .map(|rp| rp.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        })
        .collect();
    let (title, description) = if targets.len() == 1 {
        let kind = if targets[0].1 {
            t("kind.folder", "folder", locale)
        } else {
            t("kind.file", "file", locale)
        };
        // 单文件沿用旧版"只显示文件名"(basename)的口径,保持交互习惯不变;
        // 多文件才列出全部相对路径(见下方 `else` 分支),两者职责不同。
        let name = targets[0]
            .0
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| targets[0].0.display().to_string());
        (
            t_args(
                "confirm.delete.title_single",
                "Delete {kind} \"{name}\"?",
                locale,
                &[("kind", kind.as_str()), ("name", name.as_str())],
            ),
            t_args(
                "confirm.delete.desc_single",
                "The {kind} will be moved to the Trash. You can restore it from the Trash.",
                locale,
                &[("kind", kind.as_str())],
            ),
        )
    } else {
        let list = rel_paths
            .iter()
            .map(|r| format!("• {r}"))
            .collect::<Vec<_>>()
            .join("\n");
        (
            t_args(
                "confirm.delete.title_multi",
                "Delete {count} files/folders?",
                locale,
                &[("count", targets.len().to_string().as_str())],
            ),
            t_args(
                "confirm.delete.desc_multi",
                "{list}\n\nThese files/folders will be moved to the Trash. You can restore them from the Trash.",
                locale,
                &[("list", list.as_str())],
            ),
        )
    };
    Some(byteui::feedback::dialog::ConfirmDialog {
        icon: Some(byteui::interaction::icons::IconKind::CircleAlert),
        title,
        description,
        cancel_label: t("confirm.delete.cancel", "Cancel", locale),
        cancel_msg: Message::DeleteCancel,
        confirm_label: t("confirm.delete.confirm", "Delete", locale),
        confirm_msg: Message::DeleteConfirm,
        close_msg: Some(Message::DeleteCancel),
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 8.0,
    })
}

