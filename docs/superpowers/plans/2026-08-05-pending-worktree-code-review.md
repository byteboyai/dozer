# 当前工作区未提交改动代码审查（2026-08-05）

> **范围说明（重要）**：这份审查对应的是当前 `main` 工作区里**尚未提交**的改动，
> **不是** H0 项目中心的实现——`docs/superpowers/plans/2026-08-05-dozer-h0-project-center.md`
> 里的 8 个任务目前一个字符都还没落地（`workspace.rs` 里 `home_page`/`top_bar`/
> `Message` 枚举等仍是改造前的旧版本，`git log` 上也没有任何相关提交）。
>
> 审查期间这份工作区的内容还在实时变化（同一次审查过程中 `workspace.rs` 的
> diff 从 +18/-5 长到了 +97/-46），说明负责这部分的 agent 是**直接在这个工作
> 目录里改**，不是在单独的分支/worktree 里跑——这本身也是一条需要注意的地方，
> 见文末"给协作流程的建议"。本文档基于 2026-08-05 22:05 左右的一次快照，实际
> 落地前请重新 `git diff` 确认是不是这个状态。

## 严重问题（阻断构建，建议最先处理）

### 1. `main` 分支当前无法从干净检出构建（macOS，本项目唯一在跑的目标平台）

- **现象**：`crates/dozer-app/src/main.rs`（`center_traffic_lights`，`#[cfg(target_os = "macos")]`）
  在 HEAD 就已经在用 `objc2_app_kit::{NSView, NSWindowButton}` 和
  `raw_window_handle::{HasWindowHandle, RawWindowHandle}`，但 HEAD 的
  `crates/dozer-app/Cargo.toml` **没有声明** `objc2`/`objc2-app-kit`/
  `raw-window-handle` 这三个依赖（根 `Cargo.toml`、`Cargo.lock` 的 `dozer-app`
  依赖列表里也确认没有）。已用 `git show HEAD:...` 核实。
- **谁引入的**：提交 `e82a8ca fix(dozer-app): 交通灯改按原生基线绝对定位,修复多次
  resize 后彼此错位` 加了这段 `main.rs` 代码，但没有同步提交 `Cargo.toml`。
- **现状**：当前工作区里**已经有**修复这个问题的改动——`Cargo.toml` 未提交的 diff
  里加了这三行依赖——但它混在其他人正在做的、跟这个 bug 完全无关的改动里
  （见下文"进行中的改动"），还没被单独提交。也就是说：如果现在有人从 `main`
  干净 clone 一份跑 `cargo build`，会直接编译失败；只是因为本机 target 目录/
  Cargo.lock 缓存了已解析的依赖图，本地增量构建才"看起来没事"。
- **建议**：不依赖任何其他改动，单独把 `Cargo.toml` 那 3 行提交成一个
  `fix(dozer-app): 补上 e82a8ca 遗漏的 objc2/raw-window-handle 依赖声明`，
  尽快让 `main` 能重新从干净检出构建。这条不需要等其他 in-progress 功能做完。

## 进行中的改动一览（截至快照时刻，均未提交）

| 文件 | 内容 | 状态 |
|---|---|---|
| `term_model.rs` + `term_view.rs` | 终端鼠标上报模式下，滚轮转发成 xterm 鼠标转义序列（修 claude 等 TUI 进 alt screen 后滚轮"滚不动"的问题） | 编译通过、新增测试全过、clippy 干净 |
| `project.rs` + `workspace.rs`(部分) | `FileTree::reload_from_disk` + 项目树右键菜单"从磁盘重新加载" | 编译通过、新增测试全过 |
| `workspace.rs`(部分) | 左右图标栏按钮加 hover 态（`RailButton`/`App.rail_hovered`/`MouseArea::on_enter/on_exit`），以及"最后一个展开的面板区不能被收起"的保护逻辑 | 编译通过，但有 `cargo fmt` 违规（见下），且新逻辑没有单测 |
| `icons.rs` + `assets/icons/home.svg` + 新增 `refresh-cw.svg` | 新增 `IconKind::RefreshCw`；`home.svg` 换成另一版 Lucide house 图形（配合上面的 reload 菜单项） | 与 H0 计划里要用的 `IconKind::Home`/`IconKind::Search` 无冲突 |
| `Cargo.toml`/`Cargo.lock` | 补 `objc2`/`objc2-app-kit`/`raw-window-handle`（见上面严重问题） | 本身是对的，只是该单独提交 |
| `design/` 下多个旧图标/logo 文件被删，新增两张 jpeg | App 图标/品牌素材重做 | 搜了全仓没有代码/构建脚本引用这些旧文件路径，删除本身看起来安全 |

已跑过 `cargo build`、`cargo test -p dozer-app`（231 个测试全过）、
`cargo clippy -p dozer-app --all-targets -- -D warnings`（干净）验证以上"编译通过"
"测试全过"的结论。

## 不合规范/有问题的地方

### 2. `cargo fmt --check` 不过（hover 态那部分改动引入）

`rail_icon_button` 里新写的两处三元表达式没有跑过 `cargo fmt`：

```rust
// 现状(未格式化):
let color = if active || hovered { theme::GOLD } else { theme::DIM };
...
color: if active { theme::GOLD } else { Color::TRANSPARENT },
```

`cargo fmt -p dozer-app -- --check` 会要求把它们展开成多行 if/else。项目根
`CLAUDE.md`"构建与测试"一节明确写了 `cargo clippy --all-targets && cargo fmt`
是标准流程的一部分——提交前应该跑一遍 `cargo fmt -p dozer-app` 再交。

### 3. "从磁盘重新加载"菜单项挂错了地方

`context_menu_popup` 里新加的这一项：

```rust
items.push(menu_item(
    icons::IconKind::RefreshCw,
    "从磁盘重新加载",
    Message::ProjectTreeReloadFromDisk,
));
```

加在函数最后，**不受 `if menu.is_dir { ... }` 门控**，也不吃 `menu.target`——
也就是说右键**任意一个文件行**（不只是目录行）都会看到这个入口，而
`Message::ProjectTreeReloadFromDisk` 的语义（连同它自己的文档注释）是"不管
点的是谁，重读整棵树"。这跟菜单里其余每一项（新建/复制/粘贴/删除/重命名/
复制路径/在 Finder 中打开）都精确作用于 `menu.target` 的语义不一致：用户在
一个深层文件上右键，看到一条跟这个文件毫无关系、实际是"刷新整棵树"的全局
操作，容易误解成"只刷新这个文件/它所在目录"。

建议：要么只在目录节点（或项目树根节点）的菜单里出现，要么干脆挪出逐行右键
菜单，放到项目树面板自己的工具栏/标题栏上——这是一个"面向整棵树"的操作，
放进逐行菜单本身就是错的容器。

### 4. 新的"最后一个展开区不能被收起"逻辑没有可单测的纯函数,也没有测试

```rust
LeftIconSelect(v) => {
    if app.left_view == v {
        if !self.right_collapsed {
            self.left_collapsed = !self.left_collapsed;
        }
    } else { ... }
}
// RightIconSelect 对称
```

这是一条真实的行为分支（"两侧都收起"这个非法态被挡住），但直接写在
`update()` 的 match 分支里，没有抽成自由函数。这个代码库里同类"看起来简单
但有具体规则"的分支逻辑一贯的做法是拆成自由函数专门留出 headless 单测口
（比如 `next_active_after_close`、`restore_open_tabs`、`focus_project_tab`、
`project_dot`/`winning_agent_state` 都是这么处理的，且都在函数文档里写明
"拆成自由函数是为了能 headless 单测"）。现在这条新逻辑既没有拆出来,也没有
补对应测试(`cargo test` 跑过,231 个全是旧测试,没有新增任何覆盖这条分支的
用例)。建议至少加一对测试:"两侧都开着时点已选中图标会收起""只剩左（或右)
展开时点已选中图标不会收起,图标保持选中"。

### 5. 终端鼠标滚轮转发复用了 `Message::TermInput`,带出了不该有的副作用

```rust
Message::TermInput(bytes) => {
    ...
    self.with_focused_project(|ws, io| {
        if let Some(tab) = ws.tabs.get_mut(ws.active) {
            tab.model.scroll_to_bottom();
            tab.model.selection_clear();  // ← 键入即回底+清选区,是给"用户在打字"设计的
        }
        ws.send_input(io, bytes);
    });
}
```

滚轮在鼠标上报模式下被编码成字节，走的是这同一条 `TermInput` 路径，于是
"键入即清空当前文本选区"这条为键盘输入设计的副作用，现在滚轮也会触发——
如果终端上正好有一段本地文字选区，鼠标上报模式下滚一下轮子，选区会被
默默清掉。多数场景下问题不大(开鼠标上报的多是全屏 TUI，通常不会同时有
残留的文字选区)，但这是把"转发一个鼠标事件"和"键盘输入语义"混在了同一
个消息类型里，属于设计上的耦合，建议后续要么给鼠标上报字节走一条不带
`scroll_to_bottom`/`selection_clear` 副作用的专门消息，要么至少在这里写一句
注释说明这个副作用是可接受的、为什么。

另外一条不算 bug、但值得记一笔的不一致：鼠标上报模式打开时，只有**滚轮**
被转发给前台程序，左键点击/拖拽仍然走本地选区那条老路（`ButtonPressed`/
`CursorMoved` 分支完全没有查 `mouse_report_mode()`）。如果这次修复的目标就是
"专治滚不动"，这个范围收窄是合理的，但如果以后有人想"顺手"把点击也接上，
会发现两套鼠标事件在同一个 canvas 上一个转发一个不转发，语义不对称,需要
留意。

## 没发现问题的部分

- `FileTree::reload_from_disk` 本身的实现和两条新测试逻辑正确，覆盖了"折叠目录
  缓存刷新"和"目录被删连带展开态清理"两种情形，跑过。
- `mouse_report_mode`/`sgr_mouse`（`term_model.rs`）与既有的 `app_cursor_mode`/
  `bracketed_paste` 写法一致，测试(默认关闭/claude 启动序列后开启/关闭后复位)
  全过。
- `encode_wheel_report`（SGR 与 legacy X10 两种编码，含坐标封顶 223）逐条验证
  过测试用例的手算结果，编码正确。
- `home.svg` 换的是另一版 Lucide house 线稿，`IconKind::Home` 这个符号本身没变，
  跟 H0 计划里 `dozer_home_tab`/`home_sidebar` 要用的 `icons::IconKind::Home`
  没有冲突。

## 给协作流程的建议

这些改动是直接在同一个工作目录里边改边攒的，没有分支/worktree 隔离，审查过程
中内容还在实时变化。这个仓库 CLAUDE.md 明确要求"一期范围以规格为准"、
"违反即错"的裁决制度，多个 agent/会话同时在同一份未提交的 `workspace.rs` 上
改，一旦谁提交晚了、谁的编辑器缓存旧了，很容易互相覆盖对方的改动——目前
`workspace.rs` 一份文件里已经叠了三个互不相关的功能(hover 态、reload-from-disk
菜单、加上 H0 计划将来也要改同一个文件)。建议：

1. 上面第 1 条(缺依赖)不依赖别的，最先单独提交,先让 `main` 能构建。
2. 其余几块按功能边界拆成三次独立提交(hover 态一次、reload-from-disk 一次、
   图标/logo 素材一次)，而不是攒成一个大改动一起提交——方便回滚、也方便
   `git blame` 定位。
3. H0 的实现开始前，先确认这三块进行中的改动已经提交(或者协调好谁先谁后)，
   避免 H0 那 8 个任务在这份还没定稿的 `workspace.rs` 上做 Edit 时对不上。
