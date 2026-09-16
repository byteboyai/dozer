# Webview 弹层遮挡隐藏补全 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 右键菜单/输入框剪切粘贴菜单等原生浮层打开时,若它们可能压在 Files/Project/Conversations/浏览器面板的 webview 内容区之上,强制隐藏对应 webview,避免浮层被 webview 盖住看不见。

**Architecture:** wry 的子 webview 是 `ns_view.addSubview(&webview)` 加进去的原生子视图(见 `wry-0.55.1` 源码 `wkwebview/mod.rs:660-705`),子视图天然合成在父视图(iced/wgpu 渲染的那块 view)自身内容之上——这是 AppKit 图层树的固有关系,不是排序配置能调的,`CLAUDE.md` "webview 恒在 GPU 内容之上"的裁决就是这个根因的准确总结。项目里已有的应对模式(见 `app.rs::preview_desired` 里 `app_modal_open`/`tab_overflow_open`)是在 webview 的 `WebviewSpec.visible` 上按需强制置 `false`,弹层关闭后自然恢复——本 plan 不引入新架构,只是把这套已验证的"按面板种类判断是否需要隐藏"的口径,扩展覆盖到之前漏判的三类面板内浮层(Files 右键菜单、Project 链接右键菜单、Conversations agent 筛选下拉)和一类跨面板的浮层(`text_input_menu` 输入框剪切/复制/粘贴菜单,覆盖浏览器地址栏等)。

考虑过的替代方案(原生 NSMenu 替换 `crate::menu.rs`、反转 z 序让 GPU 层悬浮于透明 webview 之上)经确认后**不在本 plan 范围**——`crate::menu.rs` 是被 10 处面板复用的统一弹出菜单组件,替换成本和风险远超收益;z 序反转要求重构 winit contentView 归属和 wgpu surface 透明合成,牵动焦点路由等既有 hack,同样判定为不值当。

**Tech Stack:** Rust,iced_widget/iced_wgpu 0.14,wry 0.55.1(均为既有依赖,不新增)。

**Spec:** 无独立 spec 文档——本次改动范围小且方向已在对话中调研确认(根因见上,替代方案对比见上一段),决策直接写在本 plan 里,不单独出 spec。

## Global Constraints

- 只改 `crates/dozer-app/src/app.rs` 和 `crates/dozer-app/src/extensions/conversations.rs` 两个文件,不touch `dozer-core`/`dozer-client`/`wry` 相关 crate。
- 不新增依赖,不引入 AppKit/NSMenu 等平台专属 API。
- `cargo clippy --all-targets && cargo fmt` 必须通过(仓库 CLAUDE.md 硬性要求)。
- 按 `[[feedback-plans-use-worktree-branch]]` 的既有教训:本 plan 在独立分支/worktree 上开发,完工后经审阅再合并 main,不直接在 main 上改。

---

## File Structure

- `crates/dozer-app/src/app.rs`
  - 新增私有纯函数 `webview_hidden_by_panel_popup`(不依赖 `self`,可直接单测),紧邻 `preview_desired` 之前定义,和文件里已有的 `tab_drag_past_threshold`/`tree_drag_past_threshold` 是同一种"提取成纯函数以便测试"的写法。
  - 修改 `preview_desired`(约 4631-4691 行):`app_modal_open` 并入 `text_input_menu` 判断;循环体里新增 `panel_popup_open` 并入最终的隐藏判断。
  - 修改 `browser_desired`(约 4696-4733 行):两条分支(首页 `home_browser`/工作区内 `ws.browser`)的 spec 都按 `text_input_menu` 是否打开强制 `visible = false`。
  - `mod tests`(文件底部,约 10191 行起):新增 `webview_hidden_by_panel_popup` 的用例。
- `crates/dozer-app/src/extensions/conversations.rs`
  - `impl WorkspaceState`(约 129 行起,`search_focused` 方法旁)新增一个只读访问器 `agent_picker_open(&self) -> bool`,把私有字段 `agent_picker_open`(123 行)暴露给 `app.rs` 读取——注意这个字段名和 `Workspace` 本身(`workspace.rs:427`)上另一个同名但语义完全不同的 `agent_picker_open`(Agent 面板"＋"选人,项目级状态,无 webview)撞名,两者互不相干,不要混用。

---

## Task 1: 提取并测试 `webview_hidden_by_panel_popup` 纯函数

**Files:**
- Modify: `crates/dozer-app/src/app.rs` (新增函数,紧邻 `preview_desired` 之前,约第 4625 行处)
- Test: `crates/dozer-app/src/app.rs` 底部 `mod tests`(约 10191 行起)

**Interfaces:**
- Produces: `fn webview_hidden_by_panel_popup(kind: PanelKind, files_context_menu_open: bool, project_link_menu_open: bool, conversations_agent_picker_open: bool) -> bool` —— Task 2 直接调用这个函数。

- [ ] **Step 1: 写失败的测试**

在 `app.rs` 底部 `mod tests { use super::*; ... }` 块内追加(放在已有 `tree_drag_*` 系列用例之后即可,任意位置都行,只要在 `mod tests` 块内):

```rust
    /// Files 面板右键菜单开着时该隐藏该侧 webview——同 `tab_overflow_open`
    /// 的既有口径,只是浮层换成了文件树右键菜单(`crate::menu.rs` 文档里
    /// "唯一基准"的那个)。
    #[test]
    fn webview_hidden_by_panel_popup_files_context_menu_open() {
        assert!(webview_hidden_by_panel_popup(
            PanelKind::Files,
            true,
            false,
            false,
        ));
    }

    /// Project 面板链接行右键菜单开着时该隐藏该侧 webview。
    #[test]
    fn webview_hidden_by_panel_popup_project_link_menu_open() {
        assert!(webview_hidden_by_panel_popup(
            PanelKind::Project,
            false,
            true,
            false,
        ));
    }

    /// Conversations 面板 agent 筛选下拉开着时该隐藏该侧 webview。
    #[test]
    fn webview_hidden_by_panel_popup_conversations_agent_picker_open() {
        assert!(webview_hidden_by_panel_popup(
            PanelKind::Conversations,
            false,
            false,
            true,
        ));
    }

    /// 标志位为真,但当前面板种类对不上——不该被误伤隐藏(比如 Project
    /// 链接菜单开着,但这一侧现在显示的是 Files)。
    #[test]
    fn webview_hidden_by_panel_popup_false_when_kind_mismatches_the_open_flag() {
        assert!(!webview_hidden_by_panel_popup(
            PanelKind::Files,
            false,
            true,
            false,
        ));
    }

    /// 没有 webview 的面板种类(如 Todo)恒不隐藏,即便三个标志全为真——
    /// 这几个标志本就不该对这类面板产生任何效果。
    #[test]
    fn webview_hidden_by_panel_popup_false_for_panel_kinds_without_a_webview() {
        assert!(!webview_hidden_by_panel_popup(
            PanelKind::Todo,
            true,
            true,
            true,
        ));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app webview_hidden_by_panel_popup`
Expected: 编译失败,报 `cannot find function 'webview_hidden_by_panel_popup' in this scope`(函数还不存在)。

- [ ] **Step 3: 写最小实现**

在 `app.rs` 里紧邻 `preview_desired` 方法**之前**(即 `impl App` 块内,`preview_desired` 定义之前一行的位置,大约第 4625 行,`pub fn preview_desired(` 那一行之前)插入:

```rust
/// 判断某一侧的 webview 是否要因为**面板自身内部**的原生浮层被强制隐藏
/// ——`preview_desired` 里 `app_modal_open`/`tab_overflow_open` 两种"整块
/// 面板级"场景已经在调用处单独合并,这里补的是"面板本身还在,但面板内
/// 某个 `crate::menu` 弹层可能压住 webview 内容区"的场景:Files 面板的
/// 文件树右键菜单、Project 面板的链接行右键菜单、Conversations 面板的
/// agent 筛选下拉。原生 wry 子视图不听 iced 绘制顺序摆布,只能靠调用方
/// 显式把 `WebviewSpec.visible` 置 `false` 才能让浮层真正盖住它。
fn webview_hidden_by_panel_popup(
    kind: PanelKind,
    files_context_menu_open: bool,
    project_link_menu_open: bool,
    conversations_agent_picker_open: bool,
) -> bool {
    match kind {
        PanelKind::Files => files_context_menu_open,
        PanelKind::Project => project_link_menu_open,
        PanelKind::Conversations => conversations_agent_picker_open,
        _ => false,
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app webview_hidden_by_panel_popup`
Expected: 5 个用例全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "$(cat <<'EOF'
test(dozer-app): 提取 webview_hidden_by_panel_popup 纯函数

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: 接入 `preview_desired`/`browser_desired`,补 Conversations 访问器

**Files:**
- Modify: `crates/dozer-app/src/extensions/conversations.rs:129`(`impl WorkspaceState` 内,`search_focused` 方法旁)
- Modify: `crates/dozer-app/src/app.rs:4631-4691`(`preview_desired`)
- Modify: `crates/dozer-app/src/app.rs:4696-4733`(`browser_desired`)

**Interfaces:**
- Consumes: Task 1 产出的 `webview_hidden_by_panel_popup(kind, files_context_menu_open, project_link_menu_open, conversations_agent_picker_open) -> bool`。
- Consumes(既有,未改动): `self.files.context_menu_is_some() -> bool`(`files.rs:905`)、`self.project_link_menu: Option<ProjectLinkMenu>`(`app.rs:2353`)、`self.text_input_menu: Option<TextInputMenu>`(`app.rs:2360`)。
- Produces: `conversations::WorkspaceState::agent_picker_open(&self) -> bool`,新增的只读访问器,`ws.conversations.agent_picker_open()` 供 `app.rs` 调用。

- [ ] **Step 1: 给 `conversations::WorkspaceState` 加访问器**

在 `crates/dozer-app/src/extensions/conversations.rs` 的 `impl WorkspaceState` 块内(`search_focused` 方法紧邻处,约第 129-133 行)追加:

```rust
    /// `footer_bar` 的 agent 筛选下拉是否展开(`app.rs::preview_desired`
    /// 判断是否要强制隐藏本侧 webview 用)。注意 `Workspace` 自己另有一个
    /// **同名但完全不同语义**的 `agent_picker_open` 字段(Agent 面板"＋"
    /// 选人、项目级状态、不涉及任何 webview),两者互不相干。
    pub fn agent_picker_open(&self) -> bool {
        self.agent_picker_open
    }
```

- [ ] **Step 2: 编译确认新访问器可用**

Run: `cargo build -p dozer-app`
Expected: 编译通过(此步只加了访问器,尚无调用方,不会报未使用警告——`pub fn` 对 crate 外/其它模块可见)。

- [ ] **Step 3: 修改 `preview_desired`**

把 `crates/dozer-app/src/app.rs` 里的 `preview_desired` 方法(约 4631-4691 行)整体替换为:

```rust
    pub fn preview_desired(
        &self,
        window_width: f32,
        window_height: f32,
    ) -> Vec<(WebviewSpec, (f32, f32, f32, f32))> {
        if self.current_page == AppPage::Home {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        // `search_modal` 是满窗 SCRIM+卡片形制(见
        // `extensions/search.rs::search_modal` 注释),打开时要隐藏 webview,
        // 否则 webview 会盖住遮罩和弹窗卡片。`text_input_menu`(输入框右键
        // 剪切/复制/粘贴菜单)是屏幕空间单例、不区分左右哪一侧,和
        // `search_modal` 一样按"两侧都可能被盖住"从宽处理——比如文件树
        // 搜索框右键时,菜单向下弹出恰好压在下方的预览 webview 上。
        let app_modal_open = ws.search.is_open() || self.text_input_menu.is_some();
        let mut out = Vec::new();
        for side in [Side::Left, Side::Right] {
            let kind = match side {
                Side::Left => self.left_view,
                Side::Right => self.right_view,
            };
            let (specs, id_offset): (Vec<WebviewSpec>, usize) = match kind {
                PanelKind::Files => (ws.preview.desired_webviews(), 0),
                PanelKind::Project => (
                    ws.project_preview.desired_webviews(),
                    PROJECT_PREVIEW_ID_OFFSET,
                ),
                PanelKind::Conversations => (
                    crate::workspace::review_webview_spec(ws.review.as_ref()),
                    CONVERSATION_REVIEW_ID_OFFSET,
                ),
                _ => continue,
            };
            // tab 栏"溢出下拉"(V 按钮)向下弹,原生浮层会被本侧 webview
            // 盖住(webview 恒在 iced 内容之上)——按该侧对应的
            // `*_tab_overflow_anchor` 是否展开,同 `app_modal_open` 一并
            // 强制隐藏(见 `tab_widget::tab_overflow_menu` 文档)。
            let tab_overflow_open = match kind {
                PanelKind::Files => ws.preview_tab_overflow_anchor.is_some(),
                PanelKind::Project => ws.project_preview_tab_overflow_anchor.is_some(),
                _ => false,
            };
            // Files 右键菜单/Project 链接右键菜单/Conversations agent
            // 筛选下拉——同款"面板内浮层盖住 webview"场景,按当前面板种类
            // 分别判断(见 `webview_hidden_by_panel_popup` 文档)。
            let panel_popup_open = webview_hidden_by_panel_popup(
                kind,
                self.files.context_menu_is_some(),
                self.project_link_menu.is_some(),
                ws.conversations.agent_picker_open(),
            );
            let bounds = webview_geometry::preview_content_bounds_for(
                side,
                window_width,
                window_height,
                &self.shell_state(),
            );
            out.extend(specs.into_iter().map(|mut s| {
                s.id += id_offset;
                // 搜索弹窗/tab 溢出下拉/面板内浮层开着时,原生浮层盖住了
                // 预览区,原生 wry 子视图不听 iced 绘制顺序摆布,必须显式
                // visible=false 才能真正藏起来。
                if app_modal_open || tab_overflow_open || panel_popup_open {
                    s.visible = false;
                }
                (s, bounds)
            }));
        }
        out
    }
```

- [ ] **Step 4: 修改 `browser_desired`**

把 `crates/dozer-app/src/app.rs` 里的 `browser_desired` 方法(约 4696-4733 行)整体替换为:

```rust
    pub fn browser_desired(
        &self,
        window_width: f32,
        window_height: f32,
    ) -> Vec<(WebviewSpec, (f32, f32, f32, f32))> {
        // 地址栏右键"剪切/复制/粘贴"菜单向下弹出,恰好压在下方的浏览器
        // webview 内容区上——同 `preview_desired` 里 `text_input_menu` 的
        // 处理,原生 wry 子视图不听 iced 绘制顺序摆布,必须显式
        // visible=false 才能真正藏起来。首页(`home_browser`)和工作区内
        // (`ws.browser`)两条分支共用这一个判断。
        let text_input_menu_open = self.text_input_menu.is_some();
        if self.current_page == AppPage::Home {
            return self
                .home_browser
                .desired_webviews()
                .into_iter()
                .map(|mut s| {
                    if text_input_menu_open {
                        s.visible = false;
                    }
                    (s, (0.0, 0.0, 0.0, 0.0))
                })
                .collect();
        }
        let side = if self.left_view == PanelKind::Web {
            Side::Left
        } else if self.right_view == PanelKind::Web {
            Side::Right
        } else {
            return Vec::new();
        };
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let bounds = webview_geometry::preview_content_bounds_for(
            side,
            window_width,
            window_height,
            &self.shell_state(),
        );
        ws.browser
            .desired_webviews()
            .into_iter()
            .map(|mut s| {
                if text_input_menu_open {
                    s.visible = false;
                }
                (s, bounds)
            })
            .collect()
    }
```

- [ ] **Step 5: 编译 + 跑全量测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app`
Expected: 编译通过,全部既有测试(含 Task 1 新增的 5 条)PASS,无新增警告。

- [ ] **Step 6: `clippy`/`fmt`**

Run: `cargo clippy --all-targets && cargo fmt`
Expected: 无新增 clippy 警告;`fmt` 后无 diff(或只有本次改动范围内的格式化)。

- [ ] **Step 7: 人工验证(App 级集成逻辑无 fixture,只能手测——同既有"commit-on-blur 边缘逻辑无测试"的已知缺口)**

Run: `cargo run -p dozer-app`,手测以下四个场景,每个都确认"浮层打开瞬间该侧 webview 立刻隐藏、浮层收起瞬间立刻恢复,无残留":

1. 打开一个项目,左/右任一侧切到 Files 面板并预览一个文件(webview 可见),在文件树里右键一个文件——确认右键菜单完整可见,不被预览 webview 盖住。
2. 切到 Project 面板,预览一个链接目标(webview 可见),右键一条链接行——确认链接右键菜单完整可见。
3. 切到 Conversations 面板并打开一个会话(webview 可见),点开底部 agent 筛选下拉——确认下拉列表完整可见。
4. 切到浏览器面板(或回首页浏览器),右键地址栏——确认剪切/复制/粘贴菜单完整可见,不被下方网页内容盖住。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/extensions/conversations.rs
git commit -m "$(cat <<'EOF'
fix(dozer-app): 补全右键菜单/输入框菜单对 webview 的强制隐藏覆盖

Files 右键菜单、Project 链接右键菜单、Conversations agent 筛选下拉、
输入框剪切复制粘贴菜单(含浏览器地址栏)此前都没有触发 webview
隐藏,浮层会被恒在 iced 内容之上的原生 wry 子视图盖住看不见。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec coverage:** 本 plan 无独立 spec 文档,对照的是"Architecture"一节列出的四类漏判场景(Files 右键菜单、Project 链接右键菜单、Conversations agent 筛选下拉、输入框剪切复制粘贴菜单)——Task 1 覆盖前三类的判断逻辑与测试,Task 2 覆盖四类的实际接线(含新增的输入框菜单一类,直接并入 `app_modal_open`)以及 `browser_desired` 侧的浏览器地址栏场景。四类均有对应 Task 步骤,无遗漏。

**Placeholder scan:** 全部步骤含完整可编译代码,无 TBD/"补充"/"参考 Task N"字样。

**Type consistency:** `webview_hidden_by_panel_popup` 在 Task 1 定义与 Task 2 调用处签名一致(`kind: PanelKind, files_context_menu_open: bool, project_link_menu_open: bool, conversations_agent_picker_open: bool`);`conversations::WorkspaceState::agent_picker_open(&self) -> bool` 在 Task 2 Step 1 定义、Step 3 以 `ws.conversations.agent_picker_open()` 调用,签名一致。
