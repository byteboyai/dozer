//! 文件树右键菜单的内容组装——纯数据,不碰渲染、不碰 AppKit、不碰产品
//! 消息之外的任何东西。产品把 `MenuItem<Message>` 交给自己的菜单渲染
//! (Dozer 原生 `NSMenu`、Digger 的 iced 浮层),选中后再把 `msg` 送回
//! `update`。
//!
//! 与实际能力相关的条目由 `FilesCapabilities` 开关:产品没有的能力
//! (例如 Digger 暂时没有文件历史)整条不出现,而不是显示成灰的。

use byteui::interaction::icons::IconKind;

use crate::Message;
use crate::adapter::{FilesCapabilities, PreviewKind};
use crate::external_apps::ExternalAppsConfig;
use crate::fs::PathKind;
use crate::i18n::{t, t_args};
use std::path::Path;

/// 一条右键菜单项。形状对应 Dozer `menu_spec::MenuSpecItem`,但不含任何
/// 原生菜单类型——库只产出这个,产品自己转成 `NSMenu` / iced 浮层。
#[derive(Debug, Clone, PartialEq)]
pub enum MenuItem<M> {
    Entry {
        icon: Option<IconKind>,
        label: String,
        enabled: bool,
        msg: M,
    },
    Separator,
}

/// 右键菜单的参数集合。`is_dir/is_root/has_clipboard/is_git_repo` 相邻
/// 四个 `bool`,用具名字段避免位置传参错序。
pub struct FileContextMenuParams<'a> {
    pub target: &'a std::path::Path,
    pub is_dir: bool,
    pub is_root: bool,
    pub has_clipboard: bool,
    pub is_git_repo: bool,
    pub external_apps: &'a ExternalAppsConfig,
}

impl MenuItem<Message> {
    fn entry(icon: Option<IconKind>, label: impl Into<String>, msg: Message) -> Self {
        MenuItem::Entry {
            icon,
            label: label.into(),
            enabled: true,
            msg,
        }
    }

    fn gated(
        icon: Option<IconKind>,
        label: impl Into<String>,
        enabled: bool,
        msg: Message,
    ) -> Self {
        MenuItem::Entry {
            icon,
            label: label.into(),
            enabled,
            msg,
        }
    }
}

/// 组装右键菜单。返回的 `Vec` 已规整:
/// - 无首尾分隔线、无连续分隔线;
/// - 为空时返回空 `Vec`(调用方据此不弹菜单)。
///
/// 结构(与迁移前一致):
/// - 顶部:文件/目录共有的"添加到 Agent 上下文";目录再加搜索/新建文件/
///   新建文件夹;git 仓库里的文件加回滚/历史。
/// - 中间:复制恒在;目录加粘贴;非根目标加删除/重命名。
/// - 底部:复制名称/相对路径/绝对路径、用外部软件打开、从磁盘重新加载。
pub fn context_menu_items(
    params: FileContextMenuParams,
    caps: &FilesCapabilities,
    locale: &str,
) -> Vec<MenuItem<Message>> {
    let FileContextMenuParams {
        target,
        is_dir,
        is_root,
        has_clipboard,
        is_git_repo,
        external_apps,
    } = params;
    let target = target.to_path_buf();

    // 菜单项文案与打开动作都按扩展名查一次配置:有配置显示"用 {app} 打开"
    // 并指定 App,查不到显示"用外部软件打开"、退回系统默认。目录不走查表:
    // 文案固定"在Finder中打开",避免目录名恰好带扩展名后缀时被配置误命中。
    let open_app_name = if is_dir {
        None
    } else {
        external_apps.lookup_for_path(&target).map(str::to_string)
    };
    let open_label = match &open_app_name {
        _ if is_dir => t("menu.open_in_finder", "Open in Finder", locale),
        Some(name) => t_args(
            "menu.open_with_app",
            "Open with {name}",
            locale,
            &[("name", name.as_str())],
        ),
        None => t("menu.open_with_external", "Open with external app", locale),
    };

    // 顶部操作组:文件夹=Agent/搜索/新建;文件=Agent/回滚/历史。
    let mut top: Vec<MenuItem<Message>> = Vec::new();
    if caps.send_to_agent {
        top.push(MenuItem::entry(
            Some(IconKind::MessageSquare),
            t("menu.add_to_agent_context", "Add to Agent context", locale),
            Message::SendToAgentContext(target.clone(), is_dir),
        ));
    }
    if is_dir {
        if caps.search {
            top.push(MenuItem::entry(
                Some(IconKind::Search),
                t("menu.search", "Search", locale),
                Message::OpenSearch(target.clone(), is_dir),
            ));
        }
        top.push(MenuItem::entry(
            Some(IconKind::FilePlus),
            t("menu.new_file", "New file", locale),
            Message::NewFile(target.clone()),
        ));
        top.push(MenuItem::entry(
            Some(IconKind::FolderPlus),
            t("menu.new_folder", "New folder", locale),
            Message::NewFolder(target.clone()),
        ));
    } else if is_git_repo && caps.file_history {
        top.push(MenuItem::entry(
            Some(IconKind::Undo2),
            t("menu.rollback", "Roll back", locale),
            Message::FileHistoryRollbackPrevious(target.clone()),
        ));
        top.push(MenuItem::entry(
            Some(IconKind::FileClock),
            t("menu.history", "History", locale),
            Message::FileHistoryOpen(target.clone()),
        ));
    }

    // 中间主操作组:复制恒在;粘贴仅目录(无剪贴内容时禁用);删除/重命名
    // 仅非根目标才有。
    let mut mid: Vec<MenuItem<Message>> = Vec::new();
    mid.push(MenuItem::entry(
        Some(IconKind::Copy),
        t("menu.copy", "Copy", locale),
        Message::Copy(target.clone(), is_dir),
    ));
    if is_dir {
        mid.push(MenuItem::gated(
            Some(IconKind::ClipboardPaste),
            t("menu.paste", "Paste", locale),
            has_clipboard,
            Message::Paste(target.clone()),
        ));
    }
    if !is_root {
        mid.push(MenuItem::entry(
            Some(IconKind::Trash),
            t("menu.delete", "Delete", locale),
            Message::DeleteRequest(target.clone(), is_dir),
        ));
        mid.push(MenuItem::entry(
            Some(IconKind::Rename),
            t("menu.rename", "Rename", locale),
            Message::RenameStart(target.clone()),
        ));
    }

    // 底部工具组:纯文字(无图标)。
    let name_label = if is_dir {
        t("menu.copy_dir_name", "Copy folder name", locale)
    } else {
        t("menu.copy_file_name", "Copy file name", locale)
    };
    let bottom: Vec<MenuItem<Message>> = vec![
        MenuItem::entry(
            None,
            &name_label,
            Message::CopyPath(target.clone(), PathKind::Name),
        ),
        MenuItem::entry(
            None,
            t("menu.copy_relative_path", "Copy relative path", locale),
            Message::CopyPath(target.clone(), PathKind::Relative),
        ),
        MenuItem::entry(
            None,
            t("menu.copy_absolute_path", "Copy absolute path", locale),
            Message::CopyPath(target.clone(), PathKind::Absolute),
        ),
        MenuItem::entry(
            None,
            &open_label,
            Message::OpenWithDefault(target.clone(), open_app_name),
        ),
        MenuItem::entry(
            None,
            t("menu.reload_from_disk", "Reload from disk", locale),
            Message::ReloadFromDisk,
        ),
    ];

    let mut items = top;
    if !items.is_empty() {
        items.push(MenuItem::Separator);
    }
    items.extend(mid);
    items.push(MenuItem::Separator);
    items.extend(bottom);
    normalize(items)
}

/// 去掉首尾分隔线与连续分隔线。组装逻辑本身不产生这些,但能力开关与
/// 将来的条件分支可能让某一组整体消失,统一在这里兜底,调用方不必关心。
fn normalize(items: Vec<MenuItem<Message>>) -> Vec<MenuItem<Message>> {
    let mut out: Vec<MenuItem<Message>> = Vec::with_capacity(items.len());
    for item in items {
        if matches!(item, MenuItem::Separator)
            && (out.is_empty() || matches!(out.last(), Some(MenuItem::Separator)))
        {
            continue;
        }
        out.push(item);
    }
    while matches!(out.last(), Some(MenuItem::Separator)) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn params<'a>(
        target: &'a Path,
        is_dir: bool,
        is_root: bool,
        has_clipboard: bool,
        is_git_repo: bool,
        config: &'a ExternalAppsConfig,
    ) -> FileContextMenuParams<'a> {
        FileContextMenuParams {
            target,
            is_dir,
            is_root,
            has_clipboard,
            is_git_repo,
            external_apps: config,
        }
    }

    fn labels(items: &[MenuItem<Message>]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Entry { label, .. } => Some(label.clone()),
                MenuItem::Separator => None,
            })
            .collect()
    }

    /// 迁移前 `context_menu_spec` 在 Dozer 上的黄金标签序列(全能力)。
    /// 目标形态:目录(非根/根)、文件(git 仓库内/外)、剪贴板有无,共 6 组。
    /// 只有标签,`msg`/图标不参与比对——那些由下面的专门测试与 Dozer 侧
    /// 的既有测试各自覆盖。
    fn all_combinations() -> Vec<(bool, bool, bool, bool)> {
        let mut out = Vec::new();
        for is_dir in [true, false] {
            for is_root in [false, true] {
                for has_clipboard in [false, true] {
                    for is_git_repo in [false, true] {
                        // 文件不可能是根,跳过无意义组合,但保留剪贴板维度
                        // (文件菜单不受剪贴板影响,用来验证"不出现粘贴")。
                        if !is_dir && is_root {
                            continue;
                        }
                        out.push((is_dir, is_root, has_clipboard, is_git_repo));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn with_all_capabilities_off_no_gated_entry_appears() {
        let config = ExternalAppsConfig::default();
        let gated_labels = ["Add to Agent context", "Roll back", "History", "Search"];
        for (is_dir, is_root, has_clipboard, is_git_repo) in all_combinations() {
            let target = Path::new("/proj/a.txt");
            let items = context_menu_items(
                params(target, is_dir, is_root, has_clipboard, is_git_repo, &config),
                &FilesCapabilities::default(),
                "en",
            );
            for label in labels(&items) {
                assert!(
                    !gated_labels.contains(&label.as_str()),
                    "capability gated entry {label:?} appeared with all caps off \
                     (is_dir={is_dir} is_root={is_root} clipboard={has_clipboard} git={is_git_repo})"
                );
            }
        }
    }

    #[test]
    fn menu_never_starts_or_ends_with_a_separator_nor_has_two_in_a_row() {
        let config = ExternalAppsConfig::default();
        for caps in [FilesCapabilities::default(), FilesCapabilities::all()] {
            for (is_dir, is_root, has_clipboard, is_git_repo) in all_combinations() {
                let items = context_menu_items(
                    params(
                        Path::new("/proj/a.txt"),
                        is_dir,
                        is_root,
                        has_clipboard,
                        is_git_repo,
                        &config,
                    ),
                    &caps,
                    "en",
                );
                assert!(!items.is_empty(), "menu must not be empty");
                assert!(
                    !matches!(items.first(), Some(MenuItem::Separator)),
                    "menu starts with a separator (is_dir={is_dir})"
                );
                assert!(
                    !matches!(items.last(), Some(MenuItem::Separator)),
                    "menu ends with a separator (is_dir={is_dir})"
                );
                assert!(
                    !items.windows(2).any(|w| matches!(
                        (&w[0], &w[1]),
                        (MenuItem::Separator, MenuItem::Separator)
                    )),
                    "two separators in a row (is_dir={is_dir})"
                );
            }
        }
    }

    /// 全能力下的标签序列必须与迁移前一致。这里是逐组内联的黄金值 ——
    /// 在 Dozer 原实现上抓取后写死,改动菜单结构时此测试会明确失败。
    #[test]
    fn with_all_capabilities_on_matches_the_legacy_menu() {
        let config = ExternalAppsConfig::default();
        type Case = ((bool, bool, bool, bool), Vec<&'static str>);
        let cases: Vec<Case> = vec![
            (
                (true, false, false, false),
                vec![
                    "Add to Agent context",
                    "Search",
                    "New file",
                    "New folder",
                    "Copy",
                    "Paste",
                    "Delete",
                    "Rename",
                    "Copy folder name",
                    "Copy relative path",
                    "Copy absolute path",
                    "Open in Finder",
                    "Reload from disk",
                ],
            ),
            (
                (true, false, true, false),
                vec![
                    "Add to Agent context",
                    "Search",
                    "New file",
                    "New folder",
                    "Copy",
                    "Paste",
                    "Delete",
                    "Rename",
                    "Copy folder name",
                    "Copy relative path",
                    "Copy absolute path",
                    "Open in Finder",
                    "Reload from disk",
                ],
            ),
            (
                (true, true, false, true),
                vec![
                    "Add to Agent context",
                    "Search",
                    "New file",
                    "New folder",
                    "Copy",
                    "Paste",
                    "Copy folder name",
                    "Copy relative path",
                    "Copy absolute path",
                    "Open in Finder",
                    "Reload from disk",
                ],
            ),
            (
                (false, false, false, false),
                vec![
                    "Add to Agent context",
                    "Copy",
                    "Delete",
                    "Rename",
                    "Copy file name",
                    "Copy relative path",
                    "Copy absolute path",
                    "Open with external app",
                    "Reload from disk",
                ],
            ),
            (
                (false, false, true, true),
                vec![
                    "Add to Agent context",
                    "Roll back",
                    "History",
                    "Copy",
                    "Delete",
                    "Rename",
                    "Copy file name",
                    "Copy relative path",
                    "Copy absolute path",
                    "Open with external app",
                    "Reload from disk",
                ],
            ),
        ];
        for ((is_dir, is_root, has_clipboard, is_git_repo), expected) in cases {
            let items = context_menu_items(
                params(
                    Path::new("/proj/a.txt"),
                    is_dir,
                    is_root,
                    has_clipboard,
                    is_git_repo,
                    &config,
                ),
                &FilesCapabilities::all(),
                "en",
            );
            assert_eq!(
                labels(&items),
                expected,
                "is_dir={is_dir} is_root={is_root} clipboard={has_clipboard} git={is_git_repo}"
            );
        }
    }

    #[test]
    fn configured_extension_uses_open_with_app_label() {
        let mut config = ExternalAppsConfig::default();
        config
            .by_extension
            .insert("xlsx".to_string(), "Microsoft Excel".to_string());
        let items = context_menu_items(
            params(
                Path::new("/proj/report.xlsx"),
                false,
                false,
                false,
                false,
                &config,
            ),
            &FilesCapabilities::default(),
            "en",
        );
        let labels = labels(&items);
        assert!(labels.iter().any(|l| l == "Open with Microsoft Excel"));
        assert!(!labels.iter().any(|l| l == "Open with external app"));
    }

    #[test]
    fn paste_entry_is_disabled_without_clipboard() {
        let config = ExternalAppsConfig::default();
        for (has_clipboard, expected) in [(false, false), (true, true)] {
            let items = context_menu_items(
                params(
                    Path::new("/proj"),
                    true,
                    false,
                    has_clipboard,
                    false,
                    &config,
                ),
                &FilesCapabilities::default(),
                "en",
            );
            let paste = items
                .iter()
                .find_map(|i| match i {
                    MenuItem::Entry { label, enabled, .. } if label == "Paste" => Some(*enabled),
                    _ => None,
                })
                .expect("paste entry present for directory");
            assert_eq!(paste, expected);
        }
    }

    #[test]
    fn file_history_entries_only_for_git_repo_files() {
        let config = ExternalAppsConfig::default();
        for (is_dir, is_git_repo, expect) in [
            (false, true, true),
            (false, false, false),
            (true, true, false),
        ] {
            let items = context_menu_items(
                params(
                    Path::new("/proj/a.txt"),
                    is_dir,
                    false,
                    false,
                    is_git_repo,
                    &config,
                ),
                &FilesCapabilities::all(),
                "en",
            );
            let has_history = labels(&items).iter().any(|l| l == "History");
            assert_eq!(has_history, expect, "is_dir={is_dir} git={is_git_repo}");
        }
    }

    #[test]
    fn zh_locale_labels_are_localized() {
        let config = ExternalAppsConfig::default();
        let items = context_menu_items(
            params(Path::new("/proj"), true, false, false, false, &config),
            &FilesCapabilities::all(),
            "zh-CN",
        );
        let labels = labels(&items);
        assert!(labels.iter().any(|l| l == "新建文件"), "{labels:?}");
        assert!(labels.iter().any(|l| l == "搜索"), "{labels:?}");
        assert!(!labels.iter().any(|l| l == "New file"), "{labels:?}");
    }
}

/// 预览标签页右键菜单。只在 `caps.preview_tabs` 为真时有意义(调用方据此
/// 决定是否产生 `TabContextMenuOpen`);文件历史的条目再受
/// `caps.file_history` 门控。
pub fn tab_context_menu_items(
    kind: PreviewKind,
    path: &Path,
    is_git_repo: bool,
    external_apps: &ExternalAppsConfig,
    caps: &FilesCapabilities,
    locale: &str,
) -> Vec<MenuItem<Message>> {
    let target = path.to_path_buf();
    let open_app_name = external_apps.lookup_for_path(&target).map(str::to_string);
    let open_label = match &open_app_name {
        Some(name) => t_args(
            "menu.open_with_app",
            "Open with {name}",
            locale,
            &[("name", name.as_str())],
        ),
        None => t("menu.open_with_external", "Open with external app", locale),
    };

    let mut items: Vec<MenuItem<Message>> = vec![MenuItem::entry(
        Some(IconKind::CircleX),
        t("menu.close", "Close", locale),
        Message::TabContextMenuCloseTab(kind, target.clone()),
    )];
    items.push(MenuItem::Separator);

    let mut top: Vec<MenuItem<Message>> = Vec::new();
    if is_git_repo && caps.file_history {
        top.push(MenuItem::entry(
            Some(IconKind::Undo2),
            t("menu.rollback", "Roll back", locale),
            Message::FileHistoryRollbackPrevious(target.clone()),
        ));
        top.push(MenuItem::entry(
            Some(IconKind::FileClock),
            t("menu.history", "History", locale),
            Message::FileHistoryOpen(target.clone()),
        ));
    }
    if !top.is_empty() {
        items.extend(top);
        items.push(MenuItem::Separator);
    }

    items.extend([
        MenuItem::entry(
            None,
            t("menu.copy_file_name", "Copy file name", locale),
            Message::CopyPath(target.clone(), PathKind::Name),
        ),
        MenuItem::entry(
            None,
            t("menu.copy_relative_path", "Copy relative path", locale),
            Message::CopyPath(target.clone(), PathKind::Relative),
        ),
        MenuItem::entry(
            None,
            t("menu.copy_absolute_path", "Copy absolute path", locale),
            Message::CopyPath(target.clone(), PathKind::Absolute),
        ),
        MenuItem::entry(
            None,
            &open_label,
            Message::OpenWithDefault(target.clone(), open_app_name),
        ),
        MenuItem::entry(
            None,
            t("menu.reload_from_disk", "Reload from disk", locale),
            Message::TabReloadFromDisk(target.clone()),
        ),
    ]);
    normalize(items)
}
