//! 预览域状态机(P1d):左二 tabs、webview 期望清单。纯数据,不碰
//! wry/iced——webview 副作用由 main.rs 对照 `desired_webviews()` 差集
//! 执行(spike 约束:句柄只活在事件分发环)。
//!
//! 地址栏/URL tab(`TabKind::Web`)、`AddrTarget` 这套逻辑已经随浏览器
//! 面板扩展化(`extensions::browser::Tabs`)搬走——文件预览面板从来没有
//! 地址栏,这里只保留文件/验收两种 tab。
use std::path::PathBuf;

/// 一个预览 tab。
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
    /// 保存编辑后 `+1`,驱动 `desired_webviews()` 换 URL 逼 `sync_webview_pool`
    /// 重新 `load_url`(同 URL 不会重载,flyfish 的 WKWebView 会一直显示
    /// 保存前的旧内容)。
    pub reload_nonce: u64,
    /// 仅白名单扩展名(`is_editable_extension`)的文件 tab 有值。非空即代表这个
    /// tab 走原生渲染路径,`desired_webviews()` 据此把它从 wry 期望清单里排除。
    /// `CodeView` 没有实现 `Clone`/`PartialEq`,这也是 `PreviewTab` 摘掉这两个
    /// derive 的原因(见下方手写的 `Debug`)。
    pub editor: Option<crate::code_editor::CodeView>,
    /// 原生可编辑 tab 的"buffer 与磁盘不一致"标记:用户就地改过、还没 ⌘S 保存
    /// (或右键"刷新"/项目切换丢弃归零)为 `true`。`Blank`/`webview` tab 恒
    /// `false`。2026-09-06 原生预览不再只读,有了就地编辑就必须能显式挂脏并兜底,
    /// 否则用户会无声丢改动(见 workspace.rs 关闭/切换前的确认)。
    pub dirty: bool,
}

impl std::fmt::Debug for PreviewTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewTab")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("title", &self.title)
            .field("reload_nonce", &self.reload_nonce)
            .field("dirty", &self.dirty)
            .field("editor", &self.editor.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabKind {
    File(PathBuf),
    /// 关掉最后一个文件 tab 后自动补的空白 tab(内容区显示 Dozer 品牌标,
    /// 见 `workspace.rs::preview_pane_for`)——不进 `desired_webviews()`
    /// 期望清单,没有 wry 页面,纯 iced 原生渲染。
    Blank,
}

/// main.rs 同步 webview 的期望清单项。
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
}

/// RFC3986 严格百分号编码:unreserved(字母/数字/`-._~`)之外全部 %XX。
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `TabKind::File` → flyfish 渲染 URL 的唯一决策点。目前只有这一条渲染
/// 路径;把它从内联拼接抽成具名函数,是为将来"某些扩展名不走 flyfish、
/// 走 Acceptance 式 iced 原生 pane"的分叉预留一个函数级插入点——不引入
/// trait/注册表,YAGNI。
///
/// 文本文件(`is_editable_extension`)额外挂 `&ln=1`:host.html 读到后给
/// flyfish 的 text 渲染器开 `options.text.lineNumbers`,预览里显示行号
///(图片/PDF 渲染器不认这个 option,挂了也无副作用)。
fn flyfish_url(path: &std::path::Path) -> String {
    let mut u = format!(
        "dozer://flyfish/host.html?p={}",
        encode_component(&path.to_string_lossy())
    );
    if is_editable_extension(path) {
        u.push_str("&ln=1");
    }
    u
}

/// 读盘并按白名单扩展名构造一个**可写** `CodeView`(2026-09-06 起原生文本预览
/// 不再只读:用户可直接拖选/复制/就地编辑,配合 `Workspace` 侧的脏标记与
/// `preview_pane_save` ⌘S 落盘——见 `docs/superpowers/plans/2026-09-06-*.md`）。
/// 内容不是合法 UTF-8 时降级用 lossy 转换(不当错误);其余读取失败(不存在/权限
/// 不够等)原样透传 `std::io::Error`,调用方(`push_tab`/`bump_reload`)按现有
/// "打开失败"路径处理,不在这里新增错误类型。
///
/// 打开即程序化聚焦(键盘事件无需先点击一次即可直达编辑器)这件事挪到
/// `push_tab` 里置一次性 `pending_focus` 位——官方 `text_editor` 的焦点是
/// 真实 iced 焦点树的一部分,不能像 vendored 版本那样在构造时直接
/// `request_focus()` 拿到。
fn read_and_build_native_editor(
    path: &std::path::Path,
) -> std::io::Result<crate::code_editor::CodeView> {
    let text = std::fs::read_to_string(path).or_else(|e| {
        // 白名单扩展名但内容不是合法 UTF-8:降级用 lossy 转换,不当错误处理
        // (多数文本查看器的通行做法,见设计文档"错误处理"一节)。
        if e.kind() == std::io::ErrorKind::InvalidData {
            std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        } else {
            Err(e)
        }
    })?;
    Ok(crate::code_editor::CodeView::new(
        &text,
        extension_to_syntax(path),
        false,
    ))
}

/// "编辑/原生 code editor 预览"的适用范围。判定规则单一来源 = 语法能力:
/// `extension_to_syntax` 能给出具体语法(syntact 语法高亮)的源码类扩展名,
/// 一律收敛进原生 text editor 预览(不再落到 flyfish 只当纯文本/兜底);
/// 再加上少数"没有语法、但纯文本、进原生一样能看"的兜底扩展名。这样
/// `is_editable_extension` 与高亮器认识的语言集保持一致,不会出现"文件能高亮
/// 却一开始就进不了编辑器"的脱节(2026-09-05 用户:js/json 等代码类的文件
/// 都应由 text editor 预览)。
///
/// `.gitignore` 这类点开头、`Path::extension()` 认不出扩展名的文件沿用旧
/// 单独特判;`LICENSE`/`Makefile` 等其它**无扩展名**文件刻意不进(既有约定,
/// 见 `is_editable_extension_rejects_unknown_and_binary_like`)。`md/html` 虽
/// 命中语法分支返回 `true`,却由 `prefers_rendered_preview` 挡住默认预览(见
/// 其文档),只保留右键"编辑"入口;图片/PDF/二进制扩展名高亮器不认识、又不在
/// 纯文本兜底集,照旧交给 flyfish。
pub fn is_editable_extension(path: &std::path::Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) == Some(".gitignore") {
        return true;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // 主判据:高亮器认得出语法 → 代码类 → 原生文本/编辑器预览。顺带把
    // .md/.html 等渲染型也归为"可编辑"(右键能编辑),跟前几版行为一致。
    if extension_to_syntax(path) != "txt" {
        return true;
    }
    // 兜底:没有语法映射但确实是纯文档/配置文本的扩展名,原生预览优于 flyfish。
    matches!(
        ext.as_str(),
        "txt" | "log" | "conf" | "cfg" | "ini" | "csv" | "tsv" | "json5"
    )
}

/// 默认预览要不要走 flyfish 渲染而不是原生只读代码编辑器:目前只有
/// .md/.markdown——flyfish 内置的 markdown 渲染器能出标题/粗体/列表/代码块
/// 排版效果(GitHub 风格 `.markdown-body`),原生编辑器只能给纯文本+语法
/// 高亮,看不出排版。跟 `is_editable_extension` 是两个独立的判定:后者仍对
/// .md 返回 `true`,右键"编辑"照常能打开可写的原生编辑器
/// (`preview_edit_open_for` 独立读盘建 editor,不依赖这个 tab 当前是不是
/// 走 webview),只是**默认预览**换成渲染效果。
fn prefers_rendered_preview(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "md" | "markdown" | "html" | "htm"
    )
}

/// html/htm 走真实 `file://` URL 直接加载,不经 flyfish——flyfish 的渲染
/// 器把 html/htm 也归进它自己的通用文本/源码管线(不是当网页渲染),给它
/// 加 `prefers_rendered_preview` 只是换个地方显示源码,达不到"像 md 一样
/// 渲染出效果"的目的(核心原则见 CLAUDE.md:预览应该让用户看到 AI 产出的
/// 实际效果)。让 wry 直接加载文件本身的 `file://` URL,WKWebView 按普通
/// 网页处理,相对路径引用的 css/js/图片按文件所在目录自然解析,不用额外
/// 起服务。`p`(每段路径分量分别编码,保留 `/` 分隔符,不能直接套
/// `encode_component` 整段编码——那会把 `/` 也转义掉,破坏 URL 结构)。
fn file_url(path: &std::path::Path) -> String {
    let encoded_segments: Vec<String> = path
        .to_string_lossy()
        .split('/')
        .map(encode_component)
        .collect();
    format!("file://{}", encoded_segments.join("/"))
}

/// `TabKind::File` → wry 期望加载的 URL,按扩展名分派两条渲染路径。
fn preview_url(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => file_url(path),
        _ => flyfish_url(path),
    }
}

#[derive(Default)]
pub struct PreviewPane {
    tabs: Vec<PreviewTab>,
    active: usize,
    next_id: usize,
    /// 新建原生编辑器 tab 时置位的一次性程序化聚焦标记——`CodeView` 的焦点
    /// 是真实 iced 焦点树的一部分,构造时不能直接拿到,要等下一帧
    /// `UserInterface::build` 之后由 main.rs 用 `operation::focusable::focus`
    /// 强制聚焦(同项目树行内编辑/Todo 内容编辑的既有手法)。
    pending_editor_focus: bool,
}

impl PreviewPane {
    pub fn tabs(&self) -> &[PreviewTab] {
        &self.tabs
    }

    pub fn active_idx(&self) -> usize {
        self.active
    }

    /// 读走(消费式)一次性程序化聚焦标记。
    pub fn take_pending_editor_focus(&mut self) -> bool {
        std::mem::take(&mut self.pending_editor_focus)
    }

    /// 当前激活 tab 若走原生渲染,返回其编辑器的 `widget::Id`(供
    /// `operation::focusable::focus` 定位)。
    pub fn active_editor_focus_id(&self) -> Option<iced_widget::core::widget::Id> {
        self.tabs
            .get(self.active)
            .and_then(|t| t.editor.as_ref())
            .map(|e| e.focus_id())
    }

    pub fn open_path(&mut self, path: PathBuf) -> usize {
        // 同一文件已开则切过去,不重复开 tab（验收反馈）。
        if let Some((idx, tab)) = self
            .tabs
            .iter()
            .enumerate()
            .find(|(_, t)| t.kind == TabKind::File(path.clone()))
        {
            let id = tab.id;
            self.active = idx;
            return id;
        }
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.push_tab(TabKind::File(path), title)
    }

    fn push_tab(&mut self, kind: TabKind, title: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let editor = match &kind {
            TabKind::File(path)
                if is_editable_extension(path) && !prefers_rendered_preview(path) =>
            {
                read_and_build_native_editor(path).ok()
            }
            _ => None,
        };
        // 新建原生编辑器 tab:键盘事件无需先点击一次即可直达编辑器(见
        // `pending_editor_focus` 文档)。
        if editor.is_some() {
            self.pending_editor_focus = true;
        }
        self.tabs.push(PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
            editor,
            dirty: false,
        });
        self.active = self.tabs.len() - 1;
        id
    }

    /// 当前激活 tab 若是文件(webview)则返回其 id(=webview 池的 key)。
    /// 原生渲染 tab(有 `editor`)返回 `None`——它不进 webview 池。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs
            .get(self.active)
            .filter(|t| t.editor.is_none())
            .map(|t| t.id)
    }
    /// 手动点 tab / 打开时切到已存在 tab。切到**另一个**文件 tab 时,若目标
    /// 走 wry 路径(没有原生 editor)则顺手推进 `reload_nonce`,让 webview
    /// 切回来时重新 `load_url` 读到磁盘最新内容(同右键"刷新"的机制)。原生
    /// editor tab 不动——它重载会重建实例、丢滚动/只读态,按品鉴保留(用户在
    /// 别处手动确认过:原生 tab 切回不自动重载)。点当前已激活 tab 是 no-op。
    pub fn select(&mut self, idx: usize) {
        if idx >= self.tabs.len() || idx == self.active {
            return;
        }
        let is_webview_file = matches!(
            &self.tabs[idx].kind,
            TabKind::File(_) if self.tabs[idx].editor.is_none()
        );
        self.active = idx;
        if is_webview_file {
            let id = self.tabs[idx].id;
            self.bump_reload(id);
        }
    }

    /// 项目切换清理专用:真清空,不补 `Blank` 占位 tab——`close()` 的自动
    /// 补位是给"用户手动关到没了"这个交互场景用的,项目切换是"整个 pane
    /// 要换主人",旧项目的空白占位 tab 没必要带过去。调用方(`Workspace::
    /// close_all_tabs_for_switch`)原来是 `while !tabs().is_empty() {
    /// close(0) }`,`close()` 加了自动补位后那个循环会死循环,所以专门
    /// 开一个不走补位逻辑的清空方法。
    pub fn clear_all(&mut self) {
        self.tabs.clear();
        self.active = 0;
    }

    /// 关掉一个 tab。若这是最后一个,不留空——立刻补一个 `TabKind::Blank`
    /// 空白 tab(浏览器"关到只剩新标签页"那种体验),`push_tab` 顺带把
    /// `active` 指过去,不用再手动纠正。
    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.push_tab(TabKind::Blank, "空白".into());
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if idx < self.active {
            self.active -= 1;
        }
    }

    /// 拖拽换位:把 `from` 处的 tab 移到 `to` 处,并同步 `active` 下标。`from`
    /// 与 `to` 相等或越界时是 no-op。返回移动前后 `active` 是否变化(调用方
    /// 据此决定是否要重排 index-keyed 的 hover 动画键)。
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if self.active == from {
            self.active = to;
        } else if from < self.active && to >= self.active {
            // 源在激活项左侧,且目标落到了激活项右侧/身上——激活项左移一位。
            self.active -= 1;
        } else if from > self.active && to <= self.active {
            // 源在激活项右侧,且目标落到了激活项左侧/身上——激活项右移一位。
            self.active += 1;
        }
    }

    /// webview 期望清单:每文件 tab 一个,仅激活者可见(设计 D2)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.editor.is_none())
            .filter_map(|(idx, tab)| {
                // `Blank` 没有 wry 页面(内容区是纯 iced 渲染的 Dozer 品牌标),
                // 不进期望清单——`sync_webview_pool` 据此不会为它创建 webview。
                let TabKind::File(path) = &tab.kind else {
                    return None;
                };
                let mut u = preview_url(path);
                if tab.reload_nonce > 0 {
                    // flyfish URL 已经带 `?p=...` 查询串,html 的 file:// URL
                    // 还没有——按 URL 是否已有查询串决定用 `&` 还是 `?` 起头。
                    let sep = if u.contains('?') { '&' } else { '?' };
                    u.push_str(&format!("{sep}_r={}", tab.reload_nonce));
                }
                Some(WebviewSpec {
                    id: tab.id,
                    url: u,
                    visible: idx == self.active,
                })
            })
            .collect()
    }

    /// 按 `PreviewTab.id` 把某个原生 tab 标脏(当且仅当其编辑器收到过"改正文"
    /// 的 Action 时由 Workspace 转发层调用;见 `preview_tab_editor_event`)。
    /// tab 不存在/非原生时 no-op。
    pub fn mark_dirty_by_id(&mut self, tab_id: usize) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id)
            && tab.editor.is_some()
        {
            tab.dirty = true;
        }
    }

    /// 按 `PreviewTab.id` 清除脏标记(⌘S 成功落盘后调用)。
    pub fn clear_dirty_by_id(&mut self, tab_id: usize) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.dirty = false;
        }
    }

    /// 按 tab id 取该 tab 的原生 editor 可变引用。tab 不存在或该 tab 走 wry
    /// 路径(没有 editor)都返回 `None`。main.rs 把 `Message::PreviewEditorEvent`
    /// 转发的 `Action` 用它路由给正确的 tab。
    pub fn editor_mut(&mut self, tab_id: usize) -> Option<&mut crate::code_editor::CodeView> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.editor.as_mut())
    }

    /// 右键菜单"刷新"落地:按 tab 下标(与 `close`/`select`/`edit_open`
    /// 同一 vec 位置约定)重载该 tab 的内容。下标越界是 no-op(菜单目标已
    /// 被拖拽/关闭换位时防御)。内部转成 `bump_reload`(按 id 定位)——
    /// 原生 (`editor`) tab 直接读盘重建只读编辑器,webview tab 走
    /// `reload_nonce` 计数逼 `desired_webviews()` 换 URL 重新 `load_url`。
    pub fn reload_at(&mut self, idx: usize) {
        let Some(tab_id) = self.tabs.get(idx).map(|t| t.id) else {
            return;
        };
        self.bump_reload(tab_id);
    }

    /// 编辑保存后调用:按 `PreviewTab.id` 找到对应 tab,推进 reload。原生
    /// (有 `editor`)tab 直接读盘重建编辑器实例(`bump_reload` 路径),wry
    /// tab 走 `reload_nonce` 计数(驱动 `desired_webviews()` 换 URL)。未知
    /// id 是 no-op(tab 可能已被关闭)。
    pub fn bump_reload(&mut self, tab_id: usize) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        if tab.editor.is_some() {
            // 原生 tab:只读态没有光标/undo 历史值得跨重建保留,直接读盘换新
            // 实例比"原地更新缓冲区"更简单可靠。读取失败保留旧 editor 不动
            // (比闪成空白/丢内容更安全的降级)。`Blank` tab 恒 `editor: None`
            // (见 `push_tab`),这个分支实际到不了,`else` 只是满足穷尽性。
            let TabKind::File(path) = &tab.kind else {
                return;
            };
            if let Ok(fresh) = read_and_build_native_editor(path) {
                tab.editor = Some(fresh);
                // 读盘重建 = 重载/刷新:buffer 回到磁盘态,原先的就地改动(若有)
                // 一并丢弃,脏标记清零(可写后"刷新"会丢未保存改动——右键刷新前
                // 是否弹确认由调用方 handler 决定,清空这里是为了状态自洽)。
                tab.dirty = false;
            }
        } else {
            tab.reload_nonce += 1;
        }
    }

    /// 外部文件系统变化后,按变更路径集跟进预览:只重载**走 wry 的 webview**
    /// 文件 tab(其路径命中任一 `changed`),推进 `reload_nonce` 让它 `load_url`
    /// 读到磁盘最新内容。原生 editor tab **不**动——自动重载会重建实例、丢
    /// 滚动/只读态,与"手动点 tab 不重载原生"同一品鉴口径(见 `select`)。
    /// 路径比对先做逐字节精确匹配,匹配不到再对 tab 路径 canonicalize 后比
    /// 一次(`notify` 递的是规范实路径,而 tab 路径可能来自未规范化的点击)。
    /// 空变更集 / 无命中 tab 都是 no-op。
    pub fn reload_webviews_for(&mut self, changed: &[PathBuf]) {
        if changed.is_empty() {
            return;
        }
        let mut matched: Vec<usize> = self
            .tabs
            .iter()
            .filter(|t| t.editor.is_none())
            .filter_map(|t| match &t.kind {
                TabKind::File(path)
                    if changed.iter().any(|c| c == path)
                        || std::fs::canonicalize(path)
                            .map(|p| changed.iter().any(|c| c == &p))
                            .unwrap_or(false) =>
                {
                    Some(t.id)
                }
                _ => None,
            })
            .collect();
        matched.sort();
        matched.dedup();
        for id in matched {
            self.bump_reload(id);
        }
    }
}

/// 构造一份 ByteBoy2077 的 syntect 语法主题,让语法高亮的 token 颜色
/// (关键字/字符串/注释/类型/函数名……)融入 Dozer 配色。喂给
/// `code_editor::highlighter::Highlighter`(自实现的 `text::Highlighter`,
/// 不用 `iced_highlighter` 自带的 5 个内置主题——那是个封闭枚举,没有
/// "传入任意 syntect Theme" 的公开口子)。
///
/// 编辑器 chrome(背景/文本/选区色)对齐 ByteBoy2077 配色的逻辑挪到
/// `code_editor::editor_style`——官方 `text_editor::Style` 字段比这份
/// syntect 主题简单得多,没有 gutter/滚动条/右键菜单的概念(那些 UI 官方
/// widget 本来就不画,gutter 是 `code_editor` 自建的 canvas)。
///
/// 配色唯一真相源是终端 16 色面板(`term_model::ANSI16`)与终端默认前景
/// (`term_model::default_fg_rgb`)——编辑器里展示的语法色因此与终端里
/// 同级角色(字符串/关键字/注释/类型/函数……)观感一致,不会出现"编辑
/// 器一套饱和霓虹、终端一套灰调"的割裂。金 `#F2D94E`(ANSI Yellow)是
/// 甲方动作专属,不用于语法着色——需要"奶油黄"角色时用 BrightYellow。
pub(crate) fn dozer_syntax_theme() -> syntect::highlighting::Theme {
    use std::str::FromStr;
    use syntect::highlighting::{Color, ScopeSelectors, StyleModifier, ThemeItem};

    /// `(r,g,b)` -> syntect `Color`(alpha 固定 255)。
    fn c(rgb: (u8, u8, u8)) -> Color {
        Color {
            r: rgb.0,
            g: rgb.1,
            b: rgb.2,
            a: 255,
        }
    }
    /// 终端 16 色面板第 `idx` 项(下标见 `term_model::ANSI16`)。
    fn ansi(idx: usize) -> (u8, u8, u8) {
        crate::term_model::ansi16_color(idx).expect("ANSI16 静态色表必须完整")
    }
    /// 终端默认前景。
    fn body() -> (u8, u8, u8) {
        crate::term_model::default_fg_rgb()
    }
    /// 单条 scope 着色规则。
    fn scope(s: &str, rgb: (u8, u8, u8)) -> ThemeItem {
        ThemeItem {
            scope: ScopeSelectors::from_str(s).expect("静态 scope 字符串必须合法"),
            style: StyleModifier {
                foreground: Some(c(rgb)),
                background: None,
                font_style: None,
            },
        }
    }

    // 从终端色板取的语法角色(下标即 ANSI16 下标):
    //   1 Red         2 Green       4 Blue(类型/类)    6 Cyan(关键字)
    //   7 White(奶油) 8 BrightBlack(注解/屏弱)            9 BrightRed(删除)
    //  11 BrightYellow(橙/数值/属性)                       12 BrightBlue(函数)
    //  14 BrightCyan / 2 Green(插入)
    const COMMENT: usize = 8; // BrightBlack #6B7F8F
    const CREAM: usize = 7; //  White #FFE5B4
    const GREEN: usize = 2; //  Green #1AD585
    const CYAN: usize = 6; //  Cyan   #47DEF0
    const PURPLE: usize = 4; // Blue   #9580FF
    const RED: usize = 1; //  Red    #FF6E6E
    const ORANGE: usize = 11; // BrightYellow #FFF3B0(非甲方金)
    const FUNCTION: usize = 12; // BrightBlue   #B5A5FF

    syntect::highlighting::Theme {
        name: Some("ByteBoy2077".to_string()),
        author: Some("Dozer".to_string()),
        settings: syntect::highlighting::ThemeSettings {
            foreground: Some(c(ansi(CREAM))),
            background: Some(c((0x0a, 0x0e, 0x16))),
            ..Default::default()
        },
        scopes: vec![
            scope("comment", ansi(COMMENT)),
            scope("comment.line", ansi(COMMENT)),
            scope("comment.block", ansi(COMMENT)),
            scope("string", ansi(GREEN)),
            scope("string.quoted", ansi(GREEN)),
            scope("string.regexp", ansi(ORANGE)),
            scope("constant.numeric", ansi(ORANGE)),
            scope("constant.language", ansi(CYAN)),
            scope("constant", ansi(ORANGE)),
            scope("keyword", ansi(CYAN)),
            scope("keyword.control", ansi(CYAN)),
            scope("keyword.operator", body()),
            scope("keyword.other", ansi(CYAN)),
            scope("storage", ansi(CYAN)),
            scope("storage.type", ansi(CYAN)),
            scope("storage.modifier", ansi(CYAN)),
            scope("entity.name.function", ansi(FUNCTION)),
            scope("entity.name.type", ansi(PURPLE)),
            scope("entity.name.class", ansi(PURPLE)),
            scope("entity.name.struct", ansi(PURPLE)),
            scope("entity.name.enum", ansi(PURPLE)),
            scope("entity.name.trait", ansi(PURPLE)),
            scope("entity.name.namespace", body()),
            scope("entity.name", ansi(CREAM)),
            scope("entity.name.variable", ansi(CREAM)),
            scope("variable", ansi(CREAM)),
            scope("variable.parameter", ansi(CREAM)),
            scope("variable.language", ansi(CYAN)),
            scope("support.function", ansi(FUNCTION)),
            scope("support.type", ansi(PURPLE)),
            scope("support.class", ansi(PURPLE)),
            scope("support.constant", ansi(ORANGE)),
            scope("support.variable", ansi(CREAM)),
            scope("punctuation", body()),
            scope("punctuation.definition", body()),
            scope("punctuation.separator", body()),
            scope("punctuation.terminator", body()),
            scope("meta", ansi(CREAM)),
            scope("operator", body()),
            scope("markup.inserted", ansi(GREEN)),
            scope("markup.deleted", ansi(RED)),
            scope("markup.changed", ansi(ORANGE)),
            scope("markup.heading", ansi(CYAN)),
            scope("markup.bold", ansi(CREAM)),
            scope("markup.italic", ansi(CREAM)),
            scope("invalid", ansi(RED)),
            scope("invalid.deprecated", ansi(ORANGE)),
            scope("tag", ansi(CYAN)),
            scope("attribute", ansi(ORANGE)),
            scope("attribute.name", ansi(ORANGE)),
            scope("attribute.value", ansi(GREEN)),
        ],
    }
}

/// 把文件扩展名映射到 `iced-code-editor` 的语法名(即 syntect 扩展名)。
/// 未知扩展名回退 "txt"(plain text),编辑器内部也会再兜底一次。
pub(crate) fn extension_to_syntax(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "bash",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "xml" => "xml",
        "sql" => "sql",
        "diff" => "diff",
        "lua" => "lua",
        "r" => "r",
        "swift" => "swift",
        "zig" => "zig",
        "dockerfile" => "dockerfile",
        "makefile" => "makefile",
        "proto" => "protobuf",
        "graphql" | "gql" => "graphql",
        "ex" | "exs" => "elixir",
        "hs" => "haskell",
        "scala" => "scala",
        _ => "txt",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn bump_reload_rebuilds_native_editor_without_bumping_nonce() {
        let path =
            std::env::temp_dir().join(format!("preview_reload_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn one() {}").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        assert!(p.tabs()[0].editor.is_some());
        let nonce_before = p.tabs()[0].reload_nonce;

        std::fs::write(&path, "fn two() {}").unwrap();
        p.bump_reload(id);

        assert_eq!(
            p.tabs()[0].reload_nonce,
            nonce_before,
            "原生 tab 的 reload 不该走 reload_nonce 计数(那是 wry URL 换参专用信号)"
        );
        assert!(
            p.tabs()[0].editor.is_some(),
            "reload 后原生 tab 应仍持有(重建后的)editor"
        );
        assert_eq!(
            p.tabs()[0].editor.as_ref().unwrap().text(),
            "fn two() {}",
            "原生 tab reload 应读入磁盘上的新内容"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn opening_native_editor_tab_sets_pending_focus() {
        // 官方 `text_editor` 的焦点是真实 iced 焦点树的一部分,构造时不能
        // 直接拿到,改成置一次性 `pending_editor_focus` 位,main.rs 下一帧
        // 用 `operation::focusable::focus` 强制聚焦(见该字段文档)。
        let path =
            std::env::temp_dir().join(format!("preview_focus_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}").unwrap();

        let mut p = PreviewPane::default();
        p.open_path(path.clone());
        assert!(
            p.take_pending_editor_focus(),
            "新建原生 tab 应置一次性聚焦位"
        );
        assert!(!p.take_pending_editor_focus(), "消费式:取走后应复位");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn opening_webview_tab_does_not_set_pending_focus() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/no_focus_test.png"));
        assert!(p.tabs()[0].editor.is_none());
        assert!(
            !p.take_pending_editor_focus(),
            ".png 走 wry,没有原生 editor,不该置聚焦位"
        );
    }

    #[test]
    fn open_path_builds_native_editor_for_whitelisted_extension_only() {
        let dir = std::env::temp_dir();
        let rs_path = dir.join(format!("preview_native_test_{}.rs", std::process::id()));
        let png_path = dir.join(format!("preview_native_test_{}.png", std::process::id()));
        std::fs::write(&rs_path, "fn main() {}").unwrap();
        std::fs::write(&png_path, [0u8; 4]).unwrap();

        let mut p = PreviewPane::default();
        p.open_path(rs_path.clone());
        p.open_path(png_path.clone());

        assert!(p.tabs()[0].editor.is_some(), ".rs 扩展名应构造原生 editor");
        assert!(
            p.tabs()[1].editor.is_none(),
            ".png 扩展名不应构造原生 editor,继续走 wry"
        );

        let specs = p.desired_webviews();
        assert_eq!(
            specs.len(),
            1,
            "原生 tab 不应出现在 wry 期望清单里,只剩 .png 那个"
        );
        assert_eq!(
            specs[0].url,
            format!(
                "dozer://flyfish/host.html?p={}",
                encode_component(&png_path.to_string_lossy())
            ),
            "剩下的唯一一条 wry 期望清单条目应该是 .png 那个,URL 编码规则同 flyfish_url"
        );

        std::fs::remove_file(&rs_path).ok();
        std::fs::remove_file(&png_path).ok();
    }

    #[test]
    fn markdown_renders_via_webview_but_stays_editable() {
        // .md 是白名单扩展名(`is_editable_extension` 仍为 true,右键"编辑"
        // 照常出现),但默认预览要走 flyfish 的 markdown 渲染器而不是原生
        // 只读代码编辑器——跟 .png 这类天然不可编辑的类型走 wry 的原因不同,
        // 这里是"能编辑但默认展示渲染效果",两个判定必须独立验证。
        let dir = std::env::temp_dir();
        let md_path = dir.join(format!("preview_markdown_test_{}.md", std::process::id()));
        std::fs::write(&md_path, "# hello\n\nworld").unwrap();

        let mut p = PreviewPane::default();
        p.open_path(md_path.clone());

        assert!(
            p.tabs()[0].editor.is_none(),
            ".md 默认预览应走 flyfish 渲染,不建原生只读 editor"
        );
        assert!(
            is_editable_extension(&md_path),
            ".md 仍应保留可编辑属性,右键“编辑”入口不受影响"
        );
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 1, ".md 现在应进 wry 期望清单");
        assert!(
            specs[0].url.contains("&ln=1"),
            "&ln=1 仍按 is_editable_extension 挂上,flyfish 对非文本渲染器会忽略该 option"
        );

        std::fs::remove_file(&md_path).ok();
    }

    #[test]
    fn html_renders_via_webview_but_stays_editable() {
        // 跟 markdown_renders_via_webview_but_stays_editable 同一套断言,
        // 验证 html 现在也走"能编辑但默认展示渲染效果"这条路径。
        let dir = std::env::temp_dir();
        let html_path = dir.join(format!("preview_html_test_{}.html", std::process::id()));
        std::fs::write(&html_path, "<h1>hello</h1>").unwrap();

        let mut p = PreviewPane::default();
        p.open_path(html_path.clone());

        assert!(
            p.tabs()[0].editor.is_none(),
            ".html 默认预览应走 wry 渲染,不建原生只读 editor"
        );
        assert!(
            is_editable_extension(&html_path),
            ".html 仍应保留可编辑属性,右键“编辑”入口不受影响"
        );
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 1, ".html 现在应进 wry 期望清单");
        assert_eq!(
            specs[0].url,
            format!("file://{}", html_path.to_string_lossy()),
            ".html 应该直接加载 file:// URL,不经 flyfish(flyfish 只会把它当源码显示)"
        );

        std::fs::remove_file(&html_path).ok();
    }

    #[test]
    fn file_url_percent_encodes_each_path_segment_but_keeps_slashes() {
        assert_eq!(
            file_url(std::path::Path::new("/tmp/a b/c.html")),
            "file:///tmp/a%20b/c.html"
        );
    }

    #[test]
    fn preview_url_dispatches_html_to_file_url_and_others_to_flyfish() {
        assert_eq!(
            preview_url(std::path::Path::new("/tmp/page.html")),
            "file:///tmp/page.html"
        );
        assert_eq!(
            preview_url(std::path::Path::new("/tmp/page.HTM")),
            "file:///tmp/page.HTM",
            "扩展名判定大小写不敏感"
        );
        assert_eq!(
            preview_url(std::path::Path::new("/tmp/notes.md")),
            flyfish_url(std::path::Path::new("/tmp/notes.md")),
            "非 html/htm 扩展名不变,仍走 flyfish"
        );
    }

    #[test]
    fn code_class_extensions_route_to_native_text_editor_preview() {
        // 2026-09-05:js/json「等代码类」文件都应由 text editor 预览,而不是
        // 落进 flyfish 当不可预览的兜底。判据单一来源=语法能力(`extension_to_syntax`
        // 非 txt),凡高亮器认得出的源码扩展名都必须能进原生预览。这里挑几个
        // 旧白名单里没有、用户常碰的源码格式逐一断言。(旧列表只有 rs/toml/md/
        // txt/json/yaml/yml/sh/py/js/ts/tsx/jsx/html/css/xml/log/conf。）
        for ext in [
            "go", "java", "kt", "c", "h", "cpp", "cc", "rb", "php", "scss", "mjs", "cjs", "sql",
            "lua", "r", "swift", "zig", "proto", "graphql", "gql", "ex", "hs", "scala", "diff",
        ] {
            assert!(
                is_editable_extension(std::path::Path::new(&format!("/tmp/code.{ext}"))),
                ".{ext} 是有语法的源码扩展名,应走原生 text editor 预览"
            );
        }
    }

    #[test]
    fn image_pdf_and_binary_exts_stay_out_of_native_editor() {
        // 图片/PDF/富媒体/压缩包不属于"代码类",必须继续留给 flyfish(或其兜底),
        // 绝不能因语法判据脱节被误塞进原生文本编辑器。
        for ext in [
            "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "pdf", "zip", "mp4",
        ] {
            assert!(
                !is_editable_extension(std::path::Path::new(&format!("/tmp/a.{ext}"))),
                ".{ext} 是二进制/媒体,不该进原生文本编辑器"
            );
        }
    }

    #[test]
    fn markdown_and_html_still_editable_but_default_preview_is_rendered() {
        // is_editable_extension 变宽(语法判据)后,md/html 必须仍被
        // prefers_rendered_preview 拉去渲染预览而不是落到原生,避免回归。
        assert!(is_editable_extension(std::path::Path::new("/tmp/a.md")));
        assert!(is_editable_extension(std::path::Path::new("/tmp/a.html")));
        assert!(prefers_rendered_preview(std::path::Path::new("/tmp/a.md")));
        assert!(prefers_rendered_preview(std::path::Path::new(
            "/tmp/a.html"
        )));
    }

    #[test]
    fn bump_reload_uses_question_mark_separator_for_file_url_html() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.html"));
        p.bump_reload(id0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "file:///tmp/a.html?_r=1",
            "file:// URL 本身没有查询串,重载参数要用 ? 起头而不是 &"
        );
    }

    #[test]
    fn open_select_close_tabs() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let id1 = p.open_path(PathBuf::from("/tmp/b.md"));
        assert_eq!(p.tabs().len(), 2);
        assert_eq!(p.active_idx(), 1, "新开 tab 即激活");
        assert_ne!(id0, id1);
        assert_eq!(p.tabs()[0].title, "a.md");
        assert_eq!(p.tabs()[1].title, "b.md");
        p.select(0);
        assert_eq!(p.active_idx(), 0);
        p.close(0);
        assert_eq!(p.tabs().len(), 1);
        assert_eq!(p.active_idx(), 0);
    }

    #[test]
    fn closing_last_tab_opens_a_blank_placeholder_instead_of_leaving_empty() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/only.md"));
        assert_eq!(p.tabs().len(), 1);
        p.close(0);
        assert_eq!(
            p.tabs().len(),
            1,
            "关掉最后一个 tab 不该留空,要自动补一个 Blank 占位 tab"
        );
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
        assert_eq!(p.active_idx(), 0);
        // Blank tab 没有 wry 页面,不该进期望清单。
        assert!(p.desired_webviews().is_empty());
    }

    #[test]
    fn closing_the_blank_placeholder_replaces_it_with_a_fresh_one() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/only.md"));
        p.close(0);
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
        // 关掉这个占位 tab 本身也不该真的清空——立刻补一个新的,行为跟
        // 浏览器"关掉唯一的新标签页"一致(还是停在一个新标签页)。
        p.close(0);
        assert_eq!(p.tabs().len(), 1);
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
    }

    #[test]
    fn clear_all_empties_tabs_without_refilling_a_blank_placeholder() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a.md"));
        p.open_path(PathBuf::from("/tmp/b.md"));
        p.clear_all();
        assert_eq!(
            p.tabs().len(),
            0,
            "项目切换清理要真清空,不是 close() 那种自动补位语义"
        );
        assert_eq!(p.active_idx(), 0);
    }

    #[test]
    fn desired_webviews_builds_urls_and_visibility() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a b.md"));
        p.open_path(PathBuf::from("/tmp/c.md"));
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 2);
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa%20b.md&ln=1"
        );
        assert!(!specs[0].visible, "非激活 tab 不可见");
        assert_eq!(
            specs[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fc.md&ln=1"
        );
        assert!(specs[1].visible);
    }

    #[test]
    fn bump_reload_appends_query_param_and_only_affects_target_tab() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let _id1 = p.open_path(PathBuf::from("/tmp/b.md"));
        p.bump_reload(id0);
        let specs = p.desired_webviews();
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=1"
        );
        assert_eq!(
            specs[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&ln=1"
        );
        p.bump_reload(id0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=2"
        );
        // 未知 id 是 no-op,不 panic。
        p.bump_reload(9999);
    }

    #[test]
    fn reload_at_refreshes_target_idx_and_ignores_out_of_bounds() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a.md"));
        p.open_path(PathBuf::from("/tmp/b.md"));
        // 按 vec 下标刷新,与 `close`/`select` 同一约定。
        p.reload_at(0);
        let specs = p.desired_webviews();
        assert_eq!(
            specs[0].url, "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=1",
            "只刷新目标下标,驱动其换 URL 重载"
        );
        assert_eq!(
            specs[1].url, "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&ln=1",
            "非目标 tab 不受影响"
        );
        // 越界下标是 no-op,不 panic(菜单目标已被拖拽/关闭换位的防御)。
        p.reload_at(999);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=1",
            "越界刷新不改状态"
        );
    }

    #[test]
    fn reload_webviews_for_hits_matching_webview_tabs_only() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a.md")); // webview
        p.open_path(PathBuf::from("/tmp/b.md")); // webview
        // 原生 editor tab(真实 temp .rs 文件)。
        let rs_path =
            std::env::temp_dir().join(format!("preview_webviews_for_{}.rs", std::process::id()));
        std::fs::write(&rs_path, "fn main() {}").unwrap();
        let _ = p.open_path(rs_path.clone());

        // 空变更集:no-op,一个都不推进。
        p.reload_webviews_for(&[]);
        assert_eq!(p.tabs()[0].reload_nonce, 0);
        assert_eq!(p.tabs()[1].reload_nonce, 0);
        assert_eq!(p.tabs()[2].reload_nonce, 0);

        // 命中 b.md:只推进 b 的 reload_nonce。
        p.reload_webviews_for(&[PathBuf::from("/tmp/b.md")]);
        assert_eq!(p.tabs()[0].reload_nonce, 0, "a.md 不受影响");
        assert_eq!(p.tabs()[1].reload_nonce, 1, "b.md 命中,webview 推进");
        let specs = p.desired_webviews();
        assert_eq!(
            specs[1].url, "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&ln=1&_r=1",
            "命中的 webview 换 URL 重载"
        );

        // 命中原生 tab 的路径:不推进(原生 editor 不自动重载)。
        p.reload_webviews_for(std::slice::from_ref(&rs_path));
        assert_eq!(
            p.tabs()[2].reload_nonce,
            0,
            "原生 editor tab 命中也不自动重载,保住滚动/只读态"
        );

        // 未命中的路径:no-op。
        p.reload_webviews_for(&[PathBuf::from("/tmp/other.md")]);
        assert_eq!(p.tabs()[1].reload_nonce, 1, "未命中不改状态");

        std::fs::remove_file(&rs_path).ok();
    }

    #[test]
    fn select_reloads_webview_tab_on_switch_but_not_same_or_native() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md")); // webview(.md 走渲染预览)
        let id1 = p.open_path(PathBuf::from("/tmp/b.md")); // webview
        assert_eq!(p.active_idx(), 1);

        // 切到另一个 webview tab:推进 reload_nonce,切回时换 URL 重载。
        p.select(0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=1",
            "切到异 tab 的 webview 要自动推进 reload_nonce"
        );
        assert_eq!(
            p.desired_webviews()[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&ln=1",
            "非目标 tab 不受影响"
        );

        // 点当前已激活的 tab:no-op,不再多推进一次。
        p.select(0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=1",
            "重复选同一 tab 不改 reload_nonce"
        );

        // 原生 editor tab 切换不重载:重开一个走原生路径的文件(whitelisted
        // 非渲染扩展,读盘建 editor),其 reload_nonce 保持 0。
        let rs_path =
            std::env::temp_dir().join(format!("preview_select_test_{}.rs", std::process::id()));
        std::fs::write(&rs_path, "fn main() {}").unwrap();
        let _id_rs = p.open_path(rs_path.clone());
        assert_eq!(p.active_idx(), 2);
        assert!(p.tabs()[2].editor.is_some(), "c.rs 应是原生 editor tab");
        p.select(1); // 切回 b.md(webview)
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&ln=1&_r=1",
            "再切回 webview 推进一次 reload"
        );
        p.select(2); // 切回 c.rs(原生)
        assert_eq!(
            p.tabs()[2].reload_nonce,
            0,
            "原生 editor tab 切回不自动重载,保住滚动/只读态"
        );
        std::fs::remove_file(&rs_path).ok();

        // 越界/原生切片语义:切到越界下标是 no-op。
        let before = p.active_idx();
        p.select(99);
        assert_eq!(p.active_idx(), before);

        // id0/id1 仍在,避免未使用告警。
        let _ = (id0, id1);
    }

    #[test]
    fn is_editable_extension_covers_common_text_types() {
        assert!(is_editable_extension(Path::new("main.rs")));
        assert!(is_editable_extension(Path::new("Cargo.toml")));
        assert!(is_editable_extension(Path::new("README.md")));
        assert!(
            is_editable_extension(Path::new("notes.TXT")),
            "大小写不敏感"
        );
        assert!(is_editable_extension(Path::new("package.json")));
        assert!(is_editable_extension(Path::new("ci.yaml")));
        assert!(is_editable_extension(Path::new("ci.yml")));
        assert!(is_editable_extension(Path::new("run.sh")));
        assert!(is_editable_extension(Path::new("app.py")));
        assert!(is_editable_extension(Path::new("index.js")));
        assert!(is_editable_extension(Path::new("index.ts")));
        assert!(is_editable_extension(Path::new("index.tsx")));
        assert!(is_editable_extension(Path::new("index.jsx")));
        assert!(is_editable_extension(Path::new("page.html")));
        assert!(is_editable_extension(Path::new("style.css")));
        assert!(is_editable_extension(Path::new("data.xml")));
        assert!(is_editable_extension(Path::new("out.log")));
        assert!(is_editable_extension(Path::new("nginx.conf")));
        assert!(
            is_editable_extension(Path::new(".gitignore")),
            "点开头的无扩展名文件要特判"
        );
    }

    #[test]
    fn is_editable_extension_rejects_unknown_and_binary_like() {
        assert!(!is_editable_extension(Path::new("logo.png")));
        assert!(!is_editable_extension(Path::new("archive.zip")));
        assert!(
            !is_editable_extension(Path::new("LICENSE")),
            "无扩展名不在白名单里"
        );
        assert!(!is_editable_extension(Path::new("Makefile")));
    }

    #[test]
    fn reopening_same_file_reuses_tab() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        p.open_path(PathBuf::from("/tmp/b.md")); // 中间插一个,把激活挪走
        assert_eq!(p.active_idx(), 1);
        let id_again = p.open_path(PathBuf::from("/tmp/a.md"));
        assert_eq!(id_again, id0, "同文件复用同一 tab");
        assert_eq!(p.tabs().len(), 2, "不新增 tab");
        assert_eq!(p.active_idx(), 0, "切回已开的那个 tab");
    }

    #[test]
    fn encode_component_is_rfc3986_strict() {
        assert_eq!(encode_component("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode_component("/a b"), "%2Fa%20b");
        assert_eq!(encode_component("你"), "%E4%BD%A0");
    }

    #[test]
    fn dirty_marker_lifecycle_for_native_tab() {
        // 原生可写 tab 就地编辑:编辑事件标脏 → ⌘S 落盘清脏(mark/clear 按 id)。
        let path =
            std::env::temp_dir().join(format!("dirty_marker_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        assert!(p.tabs()[0].editor.is_some(), "夹具应落在原生 editor 分支");
        assert!(!p.tabs()[0].dirty, "新开原生 tab 默认不脏");

        p.mark_dirty_by_id(id);
        assert!(p.tabs()[0].dirty, "收到编辑 Action 后应标脏");

        // 对 webview 形态 / 不存在 id 标脏——都该 no-op。
        let empty_id = p.next_id + 99;
        p.mark_dirty_by_id(empty_id);
        assert!(
            p.tabs()[0].dirty && p.tabs().len() == 1,
            "未知 id 标脏是 no-op,不应误标/误建"
        );

        p.clear_dirty_by_id(id);
        assert!(!p.tabs()[0].dirty, "落盘成功后应清脏");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mark_dirty_only_applies_to_native_tabs() {
        // .png 走 wry(editor.is_none());对 id 标脏应被 mark_dirty_by_id 拒掉。
        let mut p = PreviewPane::default();
        let id = p.open_path(PathBuf::from("/tmp/no_dirty_mark.png"));
        assert!(p.tabs()[0].editor.is_none());
        p.mark_dirty_by_id(id);
        assert!(!p.tabs()[0].dirty, "非原生 tab 不该被标脏");
    }

    #[test]
    fn bump_reload_discards_pending_dirty_for_native_tab() {
        // 右键"刷新"重建原生 editor 会丢弃未保存改动 → 脏标记一并清零(保存
        // 态自洽:内存 buffer 都被换掉了,不能再以"有脏"混淆后续 ⌘S)。
        let path =
            std::env::temp_dir().join(format!("dirty_reload_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn one() {}").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        p.mark_dirty_by_id(id);
        assert!(p.tabs()[0].dirty);

        std::fs::write(&path, "fn two() {}").unwrap();
        p.bump_reload(id);

        assert!(
            !p.tabs()[0].dirty,
            "原生 tab 刷新重建 editor 后,内存里的旧改动已丢弃,脏标记应复位"
        );

        std::fs::remove_file(&path).ok();
    }

    /// 防漂移锚:编辑器语法高亮的 token 颜色必须锚定到终端 16 色面板与终端
    /// 默认前景,而不是一套独立的十六进制魔数。字符串=Green、关键字=Cyan、
    /// 注释=BrightBlack、类型=Blue、函数=BrightBlue、默认前/后景=终端本色。
    fn syntax_token(theme: &syntect::highlighting::Theme, scope: &str) -> Option<(u8, u8, u8)> {
        use std::str::FromStr;
        let sel =
            syntect::highlighting::ScopeSelectors::from_str(scope).expect("测试 scope 必须合法");
        theme
            .scopes
            .iter()
            .find(|item| item.scope == sel)
            .and_then(|item| item.style.foreground)
            .map(|c| (c.r, c.g, c.b))
    }

    #[test]
    fn dozer_syntax_theme_anchored_to_terminal_palette() {
        let t = dozer_syntax_theme();
        assert_eq!(
            syntax_token(&t, "string").expect("未命中 string"),
            crate::term_model::ansi16_color(2).unwrap(),
            "字符串应锚定终端 Green"
        );
        assert_eq!(
            syntax_token(&t, "keyword").expect("未命中 keyword"),
            crate::term_model::ansi16_color(6).unwrap(),
            "关键字应锚定终端 Cyan"
        );
        assert_eq!(
            syntax_token(&t, "comment").expect("未命中 comment"),
            crate::term_model::ansi16_color(8).unwrap(),
            "注释应锚定终端 BrightBlack"
        );
        assert_eq!(
            syntax_token(&t, "entity.name.type").expect("未命中类型"),
            crate::term_model::ansi16_color(4).unwrap(),
            "类型应锚定终端 Blue"
        );
        assert_eq!(
            syntax_token(&t, "entity.name.function").expect("未命中函数"),
            crate::term_model::ansi16_color(12).unwrap(),
            "函数应锚定终端 BrightBlue"
        );
        assert_eq!(
            syntax_token(&t, "operator").expect("未命中 operator"),
            crate::term_model::default_fg_rgb(),
            "运算符应锚定终端默认前景"
        );
        assert!(
            syntax_token(&t, "string.regexp")
                .map(|(_, g, _)| g)
                .expect("未命中 regexp")
                != 0xd9,
            "regexp 不应使用甲方金 #F2D94E"
        );
    }
}
