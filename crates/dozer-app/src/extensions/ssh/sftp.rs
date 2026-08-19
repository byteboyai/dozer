//! SFTP 文件传输(阶段 3):数据模型 + 驱动逻辑,独立子模块避免
//! `extensions/ssh.rs` 继续膨胀(镜像 `extensions/project/links.rs`
//! 的既有拆分先例)。

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

/// 远程文件树状态:展开集合 + 子项缓存,镜像 `crate::project::FileTree`
/// 的形状,但独立成自己的类型——本地是同步 `std::fs::read_dir`,远程是
/// 异步 SFTP `readdir` 结果回填,两者填充时机不同,不共用同一个类型。
#[derive(Debug, Default)]
pub struct RemoteTree {
    root: String,
    expanded: HashSet<String>,
    children: HashMap<String, Vec<RemoteEntry>>,
    /// 展开但读取失败的目录 → 错误文案,渲染层据此显示"⚠ 无法读取"而
    /// 不是无限 loading。
    errors: HashMap<String, String>,
}

impl RemoteTree {
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            ..Self::default()
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// 用连接建立后 SFTP `canonicalize(".")` 解析出的真实远程 home 目录
    /// 替换构造时的占位 root(构造时还没握手,不知道远程用户的真实 home
    /// 路径;之前误用字面 `"~"` 当 root——SFTP 协议的 `opendir`/`realpath`
    /// 不做 shell 语义的 tilde 展开,服务端会原样当文件名去找,几乎总是
    /// "No such file"导致远程文件树读不出来)。同时把占位 root 的展开态
    /// 迁到新 root 上,不然树看起来像"收起"的。
    pub fn set_root(&mut self, root: String) {
        if self.expanded.remove(&self.root) {
            self.expanded.insert(root.clone());
        }
        self.root = root;
    }

    /// 展开/收起某个远程目录。已展开 → 收起(返回 `None`,不需要重新
    /// 请求)。未展开且缓存里没有 → 展开并返回 `Some(path)`(调用方据此
    /// 发起一次异步 `readdir`)。未展开但已有缓存(比如收起后再展开)
    /// → 展开但返回 `None`(不重复请求,直接用缓存)。
    pub fn toggle(&mut self, dir: &str) -> Option<String> {
        if self.expanded.remove(dir) {
            return None;
        }
        self.expanded.insert(dir.to_string());
        if self.children.contains_key(dir) {
            None
        } else {
            Some(dir.to_string())
        }
    }

    /// 异步 `readdir` 结果回填。`Err` 时记进 `errors`,不清 `expanded`
    /// (目录仍显示为"已展开",只是子项区域显示错误文案)。
    pub fn set_children(&mut self, dir: &str, result: Result<Vec<RemoteEntry>, String>) {
        self.errors.remove(dir);
        match result {
            Ok(mut entries) => {
                entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.cmp(&b.name),
                });
                self.children.insert(dir.to_string(), entries);
            }
            Err(e) => {
                self.errors.insert(dir.to_string(), e);
            }
        }
    }

    /// 渲染用可见行,形状对齐 `crate::project::TreeRow`,复用同一个行
    /// 渲染函数(见 Task 5)。深度优先遍历 `expanded` 集合,只展开
    /// `expanded` 里存在的目录。
    pub fn visible_rows(&self) -> Vec<crate::project::TreeRow> {
        let mut rows = Vec::new();
        self.push_children(&self.root, 0, &mut rows);
        rows
    }

    fn push_children(&self, dir: &str, depth: usize, out: &mut Vec<crate::project::TreeRow>) {
        let Some(entries) = self.children.get(dir) else {
            return;
        };
        for entry in entries {
            let expanded = self.expanded.contains(&entry.path);
            out.push(crate::project::TreeRow {
                path: std::path::PathBuf::from(&entry.path),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                expanded,
            });
            if entry.is_dir && expanded {
                self.push_children(&entry.path, depth + 1, out);
            }
        }
    }

    /// 某个已展开目录读取失败时的错误文案。
    pub fn error_for(&self, dir: &str) -> Option<&str> {
        self.errors.get(dir).map(String::as_str)
    }
}

/// 建一条独立 SSH 连接 + 打开 SFTP 子系统 channel。每个 SFTP tab 调用
/// 一次,不复用同一主机终端 tab 的连接(brainstorming 阶段已确认的
/// 决定)。`handle`(`russh::client::Handle`)必须存活到 `SftpSession`
/// 不再使用为止——`into_stream()` 消费了 `channel`,但 `handle` 是
/// 另一个值,调用方(`Workspace::spawn_sftp_tab`)必须把 `handle` 一起
/// 存进某个长期持有的位置(实际上留在 spawn 出去的异步任务内部),不能
/// 让它在这个函数返回后被提前析构(同阶段 2 终端连接的既有约束)。
pub(crate) async fn open_sftp_session(
    host: &super::SshHost,
    password: Option<String>,
) -> Result<
    (
        russh::client::Handle<super::TestHandler>,
        russh_sftp::client::SftpSession,
    ),
    super::SshError,
> {
    let handle = super::handshake(host, password).await?;
    let channel = handle.channel_open_session().await?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(super::SshError::Russh)?;
    let stream = channel.into_stream();
    let sftp = russh_sftp::client::SftpSession::new(stream)
        .await
        .map_err(|e| super::SshError::Sftp(e.to_string()))?;
    Ok((handle, sftp))
}

/// 一个 SFTP tab 的完整状态。挂在 `Workspace.sftp_tabs`
/// (`HashMap<host_id, SftpTabState>`),不是 `SessionTab`
/// ——没有 `TerminalModel`,是"两棵文件树 + 选中态"而不是字节流查看器。
pub struct SftpTabState {
    pub host_id: String,
    pub local_tree: crate::project::FileTree,
    pub remote_tree: RemoteTree,
    pub selected_local: Option<std::path::PathBuf>,
    pub selected_remote: Option<String>,
    pub status: Option<(String, bool)>, // 文案 + 是否为错误(true=红)
    /// 右键菜单展开态:`true` = 本地行触发,`false` = 远程行触发,配上
    /// 那一行的路径。`None` = 未展开(Task 8)。
    pub context_menu: Option<(bool, String)>,
    /// 命令通道:`None` = 连接还没建好(或已断开)。`Some` 时 UI 侧发
    /// `SftpCmd`,真正的 IO 在 `Workspace::spawn_sftp_tab` 起的异步
    /// 任务里跑。
    pub(crate) cmd_tx: Option<tokio::sync::mpsc::UnboundedSender<SftpCmd>>,
}

/// SFTP 任务内部的命令(UI 侧通过这个通道请求 IO,不直接持有
/// `SftpSession`)。
pub(crate) enum SftpCmd {
    ReadDir(String /* dir */),
    Upload {
        local: std::path::PathBuf,
        remote_dir: String,
    },
    Download {
        remote: String,
        local_dir: std::path::PathBuf,
    },
}

impl SftpTabState {
    pub fn new(host_id: String, local_root: std::path::PathBuf, remote_root: String) -> Self {
        let mut remote_tree = RemoteTree::new(remote_root.clone());
        remote_tree.toggle(&remote_root);
        Self {
            host_id,
            local_tree: crate::project::FileTree::new(local_root),
            remote_tree,
            context_menu: None,
            selected_local: None,
            selected_remote: None,
            status: None,
            cmd_tx: None,
        }
    }
}

/// PFTP tab 内部交互消息:每个变体都带 `host_id` 首字段——
/// `ws.sftp_tabs` 是 `HashMap<host_id, SftpTabState>`,`route()` 靠这个
/// 字段直接 `get_mut(&host_id)` 定位到具体 tab,不需要在多个同时打开的
/// SFTP tab 之间"猜"消息属于哪一个。
#[derive(Debug, Clone)]
pub enum Message {
    /// 异步连接结果。`Ok` 携带 `canonicalize(".")` 解析出的远程 home
    /// 绝对路径,用它替换构造时的占位 root 再触发一次根目录
    /// `readdir`;`Err` 落 `status` 错误文案(握手失败或 canonicalize
    /// 本身失败都走这条,后者复用同一条错误展示路径,不值得单独分错误
    /// 类型)。
    Connected(
        String, /* host_id */
        Result<String /* remote home 绝对路径 */, String>,
    ),
    LocalToggle(String /* host_id */, std::path::PathBuf),
    LocalSelect(String /* host_id */, std::path::PathBuf),
    RemoteToggle(String /* host_id */, String /* remote dir */),
    RemoteDirLoaded(
        String, /* host_id */
        String, /* dir */
        Result<Vec<RemoteEntry>, String>,
    ),
    RemoteSelect(String /* host_id */, String /* remote path */),
    Upload(String /* host_id */),
    Download(String /* host_id */),
    TransferResult(String /* host_id */, Result<(), String>),
    ContextMenuOpen {
        host_id: String,
        is_local: bool,
        path: String,
    },
    ContextMenuClose,
}

/// 每个 `sftp::Message` 变体都带 `host_id`,`route()` 里每个分支都是
/// "`ws.sftp_tabs.get_mut(&host_id)` 精确定位到具体 tab,再改字段/发
/// `SftpCmd`"这个统一模式。
pub(crate) fn route(
    ws: &mut crate::workspace::Workspace,
    _io: &crate::workspace::ShellIo,
    msg: Message,
) {
    match msg {
        Message::Connected(host_id, result) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else {
                return;
            };
            match result {
                // 连接建好后,cmd_tx 已经由 spawn_sftp_tab 结尾同步设置好
                // (它在异步任务 spawn 之后立刻做的,不需要等 Connected
                // 消息才设置)。用解析出的真实 home 路径替换占位 root,
                // 再触发一次根目录 readdir。
                Ok(remote_home) => {
                    state.remote_tree.set_root(remote_home.clone());
                    if let Some(tx) = &state.cmd_tx {
                        let _ = tx.send(SftpCmd::ReadDir(remote_home));
                    }
                }
                Err(e) => {
                    state.status = Some((format!("SFTP 连接失败: {e}"), true));
                }
            }
        }
        Message::LocalToggle(host_id, dir) => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.local_tree.toggle(&dir);
            }
        }
        Message::LocalSelect(host_id, path) => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.selected_local = Some(path);
            }
        }
        Message::RemoteToggle(host_id, dir) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else {
                return;
            };
            if let Some(need_fetch) = state.remote_tree.toggle(&dir)
                && let Some(tx) = &state.cmd_tx
            {
                let _ = tx.send(SftpCmd::ReadDir(need_fetch));
            }
        }
        Message::RemoteDirLoaded(host_id, dir, result) => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.remote_tree.set_children(&dir, result);
            }
        }
        Message::RemoteSelect(host_id, path) => {
            if let Some(state) = ws.sftp_tabs.get_mut(&host_id) {
                state.selected_remote = Some(path);
            }
        }
        Message::Upload(host_id) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else {
                return;
            };
            let Some(local) = state.selected_local.clone() else {
                return;
            };
            // 上传目标目录:远程树根目录(v1 简化处理,不支持"上传到
            // 当前展开的子目录"——右键菜单本身触发在本地行上,不天然
            // 带一个"目标是哪个远程目录"的选择)。
            let remote_dir = state.remote_tree.root().to_string();
            let Some(tx) = &state.cmd_tx else {
                return;
            };
            let _ = tx.send(SftpCmd::Upload { local, remote_dir });
            state.status = Some(("正在上传…".to_string(), false));
        }
        Message::Download(host_id) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else {
                return;
            };
            let Some(remote) = state.selected_remote.clone() else {
                return;
            };
            let local_dir = state.local_tree.root().to_path_buf();
            let Some(tx) = &state.cmd_tx else {
                return;
            };
            let _ = tx.send(SftpCmd::Download { remote, local_dir });
            state.status = Some(("正在下载…".to_string(), false));
        }
        Message::TransferResult(host_id, result) => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else {
                return;
            };
            state.status = Some(match &result {
                Ok(()) => ("传输完成".to_string(), false),
                Err(e) => (format!("传输失败: {e}"), true),
            });
            // 上传成功后重新拉一次远程根目录,让新文件出现在远程树里;
            // 下载成功后本地树重读根目录,让新文件出现在本地树里。
            // 两边都只刷新根目录这一层。
            if result.is_ok() {
                if let Some(tx) = &state.cmd_tx {
                    let _ = tx.send(SftpCmd::ReadDir(state.remote_tree.root().to_string()));
                }
                state.local_tree.reload_from_disk();
            }
        }
        Message::ContextMenuOpen {
            host_id,
            is_local,
            path,
        } => {
            let Some(state) = ws.sftp_tabs.get_mut(&host_id) else {
                return;
            };
            if is_local {
                state.selected_local = Some(std::path::PathBuf::from(&path));
            } else {
                state.selected_remote = Some(path.clone());
            }
            state.context_menu = Some((is_local, path));
        }
        Message::ContextMenuClose => {
            for state in ws.sftp_tabs.values_mut() {
                state.context_menu = None;
            }
        }
    }
}

/// 拼远程路径:`{remote_dir}/{name}`,处理 `remote_dir` 尾部有无 `/`。
/// 抽成纯函数给单测测字符串拼接边界。
fn remote_join(base: &str, name: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), name)
}

/// 上传本地文件到远程目录。目录上传:递归遍历本地目录逐个文件调用,
/// 远程侧对应子目录不存在时先 `create_dir`(按需创建目标路径,不是
/// 独立的 mkdir 入口——见 Global Constraints 的既有界限)。
pub(crate) async fn upload(
    sftp: &russh_sftp::client::SftpSession,
    local: &std::path::Path,
    remote_dir: &str,
) -> Result<(), String> {
    if local.is_dir() {
        let name = local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| "无效的本地路径".to_string())?;
        let target_dir = remote_join(remote_dir, &name);
        sftp.create_dir(&target_dir)
            .await
            .map_err(|e| e.to_string())?;
        let entries = std::fs::read_dir(local).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            Box::pin(upload(sftp, &entry.path(), &target_dir)).await?;
        }
        Ok(())
    } else {
        let name = local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| "无效的本地路径".to_string())?;
        let target = remote_join(remote_dir, &name);
        let bytes = std::fs::read(local).map_err(|e| e.to_string())?;
        sftp.write(target, &bytes).await.map_err(|e| e.to_string())
    }
}

/// 下载远程文件到本地目录(目标目录必须已存在——本地一侧不做隐式
/// mkdir,`local_dir` 来自 `FileTree.root()`,恒存在)。目录下载走
/// `read_dir` 递归,逻辑与 `upload` 对称。
pub(crate) async fn download(
    sftp: &russh_sftp::client::SftpSession,
    remote: &str,
    local_dir: &std::path::Path,
) -> Result<(), String> {
    let name = remote.rsplit('/').next().unwrap_or(remote);
    let metadata = sftp.metadata(remote).await.map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        let target_dir = local_dir.join(name);
        std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
        let entries = sftp.read_dir(remote).await.map_err(|e| e.to_string())?;
        for entry in entries {
            Box::pin(download(sftp, &entry.path(), &target_dir)).await?;
        }
        Ok(())
    } else {
        let bytes = sftp.read(remote).await.map_err(|e| e.to_string())?;
        std::fs::write(local_dir.join(name), bytes).map_err(|e| e.to_string())
    }
}

/// 精简版行渲染:展开箭头 + 文件夹/文件图标 + 名称。不带 git 状态染色/
/// 重命名编辑态/右键菜单(那些是 Files 面板自己的功能),本地/远程两侧
/// 共用同一份实现。`on_toggle`(仅目录行触发)/`on_select` 由调用方
/// 传入,决定发的是 `Local*` 还是 `Remote*` 消息。
fn tree_column<'a>(
    title: &'a str,
    rows: Vec<crate::project::TreeRow>,
    selected: Option<&std::path::Path>,
    error_for: impl Fn(&str) -> Option<String> + 'a,
    on_toggle: impl Fn(std::path::PathBuf) -> Message + 'a,
    on_select: impl Fn(std::path::PathBuf) -> Message + 'a,
    on_context: impl Fn(String) -> Message + 'a,
) -> iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    use iced_widget::{MouseArea, column, container, row, text};
    let mut col = column![
        text(title)
            .size(crate::theme::font::subtitle())
            .color(byteui::theme::color::current().cream)
    ]
    .spacing(4);
    for r in rows {
        let indent = "  ".repeat(r.depth);
        let icon = if r.is_dir {
            if r.expanded {
                byteui::interaction::icons::IconKind::FolderOpen
            } else {
                byteui::interaction::icons::IconKind::Folder
            }
        } else {
            byteui::interaction::icons::IconKind::FileGeneric
        };
        let is_selected = selected == Some(r.path.as_path());
        let path_for_toggle = r.path.clone();
        let path_for_select = r.path.clone();
        let path_for_context = r.path.to_string_lossy().into_owned();
        // 展开但读取失败的目录:行尾缀一个红色 ⚠ 提示文案(`errors`),让
        // 用户看清"不是加载中、是真读不出来",而不是无限转圈(plan Task 9
        // Step 5 的验收条目)。本地树没有这套状态,`error_for` 恒返回 None。
        let dir_err = if r.is_dir {
            error_for(&path_for_context)
        } else {
            None
        };
        let name_el = text(r.name.clone())
            .size(crate::theme::font::body())
            .color(if is_selected {
                byteui::theme::color::current().cream
            } else {
                byteui::theme::color::current().dim
            });
        let mut label = row![
            text(indent),
            byteui::interaction::icons::view(
                icon,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim
            ),
            name_el,
        ]
        .spacing(4)
        .align_y(iced_widget::core::alignment::Vertical::Center);
        if let Some(err) = dir_err {
            label = label.push(
                text(format!("⚠ {err}"))
                    .size(crate::theme::font::caption())
                    .color(byteui::theme::color::current().red),
            );
        }
        let row_el: iced_widget::core::Element<
            'a,
            Message,
            iced_widget::Theme,
            iced_renderer::Renderer,
        > = MouseArea::new(container(label).width(iced_widget::core::Length::Fill))
            .on_press(if r.is_dir {
                on_toggle(path_for_toggle)
            } else {
                on_select(path_for_select)
            })
            .on_right_press(on_context(path_for_context))
            .into();
        col = col.push(row_el);
    }
    container(col)
        .padding(8)
        .width(iced_widget::core::Length::FillPortion(1))
        .into()
}

/// SFTP tab 主视图:左右两栏文件树(本地项目 / 远程主机)。
pub fn sftp_pane_view<'a>(
    state: &'a SftpTabState,
) -> iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    use iced_widget::{MouseArea, column, container, row, stack, text};
    let local_rows = state.local_tree.visible_rows();
    let remote_rows = state.remote_tree.visible_rows();
    let host_id = state.host_id.clone();
    let host_id2 = state.host_id.clone();
    let trees: iced_widget::core::Element<
        'a,
        Message,
        iced_widget::Theme,
        iced_renderer::Renderer,
    > = row![
        tree_column(
            "本地机器项目文件树",
            local_rows,
            state.selected_local.as_deref(),
            |_p| None,
            move |p| Message::LocalToggle(host_id.clone(), p),
            move |p| Message::LocalSelect(host_id2.clone(), p),
            {
                let h = state.host_id.clone();
                move |path| Message::ContextMenuOpen {
                    host_id: h.clone(),
                    is_local: true,
                    path,
                }
            },
        ),
        tree_column(
            "远程主机文件树",
            remote_rows,
            state.selected_remote.as_deref().map(std::path::Path::new),
            |p| state.remote_tree.error_for(p).map(str::to_string),
            {
                let h = state.host_id.clone();
                move |p| Message::RemoteToggle(h.clone(), p.to_string_lossy().into_owned())
            },
            {
                let h = state.host_id.clone();
                move |p| Message::RemoteSelect(h.clone(), p.to_string_lossy().into_owned())
            },
            {
                let h = state.host_id.clone();
                move |path| Message::ContextMenuOpen {
                    host_id: h.clone(),
                    is_local: false,
                    path,
                }
            },
        ),
    ]
    .into();

    // 连接失败 / 上传下载进度的文案(`state.status`)之前一直只写不读,
    // 用户建连出错时树是空的又没有任何提示,跟"读不出来但看着像没做"
    // 长得一模一样。补一条状态条:错误红字、进度态用普通文字。
    let mut base_col = column![].spacing(6);
    if let Some((msg, is_err)) = &state.status {
        base_col = base_col.push(text(msg.clone()).size(crate::theme::font::caption()).color(
            if *is_err {
                byteui::theme::color::current().red
            } else {
                byteui::theme::color::current().dim
            },
        ));
    }
    let base: iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        base_col.push(trees).into();

    if state.context_menu.is_none() {
        return base;
    }

    // 右键菜单开着:垫一层全尺寸透明 MouseArea 承接"点菜单外任何地方收起"
    // (与 Files 面板 `App::view` 的 dismiss 层同款约定),上面再叠菜单本体。
    let dismiss: iced_widget::core::Element<
        'a,
        Message,
        iced_widget::Theme,
        iced_renderer::Renderer,
    > = MouseArea::new(
        container(column![])
            .width(iced_widget::core::Length::Fill)
            .height(iced_widget::core::Length::Fill),
    )
    .on_press(Message::ContextMenuClose)
    .into();
    let popup: iced_widget::core::Element<
        'a,
        Message,
        iced_widget::Theme,
        iced_renderer::Renderer,
    > = sftp_context_menu(state);

    stack![base, dismiss, popup]
        .width(iced_widget::core::Length::Fill)
        .height(iced_widget::core::Length::Fill)
        .into()
}

/// 右键菜单浮层:按 `context_menu` 的 `is_local` 决定只出现"上传"(本地行)
/// 或"下载"(远程行)——不渲染无意义的禁用态(见 plan 的 UI 简化决定)。
/// 菜单本身固定叠在面板左上角,不追光标像素定位(v1 简化,够用即可)。
/// 表面/单项样式统一走 `crate::menu`(基准即文件树右键菜单)。
fn sftp_context_menu<'a>(
    state: &'a SftpTabState,
) -> iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    use iced_widget::{column, container};
    let Some((is_local, _path)) = &state.context_menu else {
        return container(column![]).into();
    };
    let host_id = state.host_id.clone();
    let (icon, label, msg) = if *is_local {
        (
            byteui::interaction::icons::IconKind::ChevronUp,
            "上传",
            Message::Upload(host_id),
        )
    } else {
        (
            byteui::interaction::icons::IconKind::ChevronDown,
            "下载",
            Message::Download(host_id),
        )
    };
    let item: iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::menu::item(Some(icon), label, msg);
    crate::menu::shell(
        vec![item],
        iced_widget::core::Length::Fixed(crate::theme::geometry::menu_item_width()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, name: &str, is_dir: bool) -> RemoteEntry {
        RemoteEntry {
            path: path.to_string(),
            name: name.to_string(),
            is_dir,
        }
    }

    #[test]
    fn toggle_expands_and_requests_on_first_open() {
        let mut tree = RemoteTree::new("/root");
        assert_eq!(tree.toggle("/root"), Some("/root".to_string()));
    }

    #[test]
    fn set_root_migrates_expanded_state_to_new_root() {
        // 占位 root("")在构造时已展开(`SftpTabState::new` 的既有行为);
        // `Connected` 解析出真实 home 路径后调用 `set_root`,展开态要
        // 跟着迁到新 root 上,否则树在换根后看起来像被收起了。
        let mut tree = RemoteTree::new("");
        tree.toggle("");
        assert_eq!(tree.root(), "");
        tree.set_root("/home/alice".to_string());
        assert_eq!(tree.root(), "/home/alice");
        // 换根后对新 root 再 toggle 应该是"收起"(说明它确实处于展开态),
        // 不是"首次展开"(那样会返回 `Some` 触发一次多余的 readdir)。
        assert_eq!(tree.toggle("/home/alice"), None);
    }

    #[test]
    fn set_root_on_unexpanded_placeholder_does_not_force_expand() {
        let mut tree = RemoteTree::new("");
        tree.set_root("/home/alice".to_string());
        assert_eq!(tree.root(), "/home/alice");
        assert_eq!(tree.toggle("/home/alice"), Some("/home/alice".to_string()));
    }

    #[test]
    fn toggle_collapses_without_requesting() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        assert_eq!(tree.toggle("/root"), None); // 收起
    }

    #[test]
    fn toggle_reopen_with_cache_does_not_request() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/a", "a", false)]));
        tree.toggle("/root"); // 收起
        assert_eq!(tree.toggle("/root"), None); // 重新展开,缓存还在,不重复请求
    }

    #[test]
    fn set_children_sorts_dirs_before_files_then_by_name() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children(
            "/root",
            Ok(vec![
                entry("/root/z.txt", "z.txt", false),
                entry("/root/bdir", "bdir", true),
                entry("/root/a.txt", "a.txt", false),
                entry("/root/adir", "adir", true),
            ]),
        );
        let rows = tree.visible_rows();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["adir", "bdir", "a.txt", "z.txt"]);
    }

    #[test]
    fn set_children_err_recorded_and_does_not_panic_on_visible_rows() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Err("权限拒绝".to_string()));
        assert_eq!(tree.error_for("/root"), Some("权限拒绝"));
        assert!(tree.visible_rows().is_empty()); // 没有子项缓存,可见行为空,不 panic
    }

    #[test]
    fn nested_expansion_shows_grandchildren() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/sub", "sub", true)]));
        tree.toggle("/root/sub");
        tree.set_children("/root/sub", Ok(vec![entry("/root/sub/f", "f", false)]));
        let rows = tree.visible_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "sub");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].name, "f");
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn collapsed_dir_hides_its_children_from_visible_rows() {
        let mut tree = RemoteTree::new("/root");
        tree.toggle("/root");
        tree.set_children("/root", Ok(vec![entry("/root/sub", "sub", true)]));
        tree.toggle("/root/sub");
        tree.set_children("/root/sub", Ok(vec![entry("/root/sub/f", "f", false)]));
        tree.toggle("/root/sub"); // 收起
        let rows = tree.visible_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "sub");
    }

    #[test]
    fn remote_join_no_trailing_slash() {
        assert_eq!(remote_join("/home/user", "file.txt"), "/home/user/file.txt");
    }

    #[test]
    fn remote_join_trailing_slash() {
        assert_eq!(
            remote_join("/home/user/", "file.txt"),
            "/home/user/file.txt"
        );
    }

    #[test]
    fn remote_join_name_with_spaces() {
        assert_eq!(remote_join("/root", "my file.txt"), "/root/my file.txt");
    }
}
