# Dozer 外壳区域样式配置化(背景色/边框/间距)

> 状态:设计中,待用户终审。
> 需求来源:2026-07-30 对话(用户:"我觉得应该整理一下整个工作区的结构,如主导航、左右两侧图标
> 工具栏等等,以规范用色、间距")。
> 上游背景:`docs/superpowers/specs/2026-07-29-dozer-shell-icon-rail-design.md`(图标栏外壳架构,
> 已实现)、`docs/superpowers/specs/2026-07-20-dozer-shell-chrome-fidelity-design.md`(14 色令牌与
> chrome 打磨的既有约定)。

## 1. 目标与动机

`theme.rs` 的 14 个颜色值已经是统一令牌(全仓库无绕开令牌直接写色值的情况),但**间距/圆角/内
边距**没有同等的纪律——`workspace.rs` 里 `.spacing(4/6/8/10/12/16)`、`.padding([2,4]/[6,10]/
[6,12]/8/12/...)`、圆角 `6.0/8.0` 等全是散落的字面量,各写各的,没有统一量级(scale),也没有
"这个区域为什么是这个值"的可追溯性。

本设计的目标:把外壳里 12 个"区域"(主导航、左右图标栏、放大态浮层、右键菜单,以及 7 个面板
内容区)各自外层容器的**背景色、边框、内边距、子元素间距**,从散落在 `workspace.rs` 各处的字面量,
搬进一份编译期内嵌的 JSON 配置文件,做到"改一处、查得到、有名字"。

**非目标(本轮明确排除)**:
- 不改变任何现有视觉效果——这是一次纯粹的"代码搬家",改动前后运行截图应像素级一致。
- 不做运行时读盘/热更新/用户自定义主题——JSON 编译期内嵌进二进制,和现在 `theme.rs` 的 14 色
  一样是写死的,只是换一种更好维护的来源格式。
- 不覆盖区域内部控件(按钮、tab、选中/悬停态)的样式——那些留在 Rust 代码里(同上一轮刚修的
  `rail_icon_button` hover/active 那种交互态样式)。
- 不借机统一目前确实不一致的数值(比如 `project_pane` 内边距 8 和 `agent_list_pane` 内边距 12)——
  如实照搬,配置化之后想统一是后续的"改一个数字"级别的小改动,不在本轮做。

## 2. 区域清单(12 个,不是最初设想的 13 个)

排查下来"验收视图"不是独立容器——它复用 `preview_pane` 的外层 chrome,只是把内容换成验收专属的
列(`acceptance_content` 把内容 push 进 `preview_pane` 已经建好的 `content` 列,不自建容器)。因此
真正各自拥有一份容器样式的是 12 个:

**外壳骨架(5)**:
| 区域 key | 对应函数 | 现状(背景/边框/内边距/gap) |
|---|---|---|
| `top_bar` | `top_bar` | 背景 `BG`;边框 `{BORDER, width 0, radius 0}`(实际不可见);padding `[0,12]`;gap `16` |
| `left_icon_rail` | `left_icon_rail` | 背景 `PANEL`;无边框;内层 column padding `{top:16,left:6,right:6,bottom:0}`;gap `12` |
| `right_icon_rail` | `right_icon_rail` | 同 `left_icon_rail` |
| `maximize_overlay` | `maximize_overlay` | 见 §4(复合结构,scrim + 描边盒子两部分) |
| `context_menu_popup` | `context_menu_popup` | 见 §4(复合结构) |

**面板内容(7)**:
| 区域 key | 对应函数 | 现状 |
|---|---|---|
| `project_pane` | `project_pane` | 背景 `PANEL`;无边框;padding `8` |
| `preview_pane` | `preview_pane` | 背景 `PANEL`;无边框;padding `8`;gap `4` |
| `browser_pane` | `browser_pane` | 背景 `PANEL`;无边框;padding `8`;gap `4` |
| `agent_list_pane` | `agent_list_pane` | 背景 `PANEL`;无边框;padding `12`;gap `8` |
| `terminal_pane` | `terminal_pane` | 背景 `TERM_BG`;无边框;padding `8`;gap `4` |
| `conversation_list_pane` | `conversation_list_pane` | 背景 `PANEL`;无边框;padding `12`;gap `8` |
| `review_content_pane` | `review_content_pane` | 背景 `PANEL`;无边框;padding `8` |

`project_pane`/`preview_pane`/`browser_pane`/`review_content_pane` 的 8 与
`agent_list_pane`/`conversation_list_pane` 的 12 是同背景色下两种不同内边距——现存的真实不一致,
本轮如实保留(见 §1 非目标)。

## 3. Schema:每区域一份直给,不搭"预设复用"层

12 个区域里数值本来就大多独立(§2 表格里同背景色的区域内边距还不一样),说明它们并非共享一套
基准值再局部覆盖——套一层"预设 + 覆盖"的间接性对当前情况没有实际去重收益,还会让"这个区域到底
用的什么值"变得要跳两次才能看到。采用**每区域一份直给**:

```json
{
  "top_bar": {
    "background": "BG",
    "border": { "color": "BORDER", "width": 0.0, "radius": 0.0 },
    "padding": [0, 12],
    "gap": 16
  },
  "left_icon_rail": {
    "background": "PANEL",
    "border": null,
    "padding": [16, 6, 0, 6],
    "gap": 12
  },
  "preview_pane": {
    "background": "PANEL",
    "border": null,
    "padding": 8,
    "gap": 4
  }
}
```

- `background`/`border.color` 是字符串,引用 `theme.rs` 现成令牌名(`"BG"`/`"PANEL"`/`"CARD"`/
  `"BORDER"`/`"CREAM"`/`"BODY"`/`"DIM"`/`"GOLD"`/`"CYAN"`/`"GREEN"`/`"PURPLE"`/`"RED"`/新增的
  `"SCRIM"`,见 §4)——不在 JSON 里重复写十六进制,`theme.rs` 仍是颜色数值的唯一真相源。
- `padding` 支持单值 / `[v,h]` / `[top,right,bottom,left]` 三种形状,与 iced 自身 `Padding` 的
  `From` 便利写法对应,自定义 `Deserialize` 实现。
- `border` 可为 `null`(无边框,对应 `Border::default()`)。
- `gap` 对应该区域最外层内容列/行的 `.spacing(...)`。

## 4. 两个不套用通用形状的区域

- **`maximize_overlay`**:实际是"变暗遮罩层"(背景 `Color{0,0,0,0.55}`、`padding
  MAXIMIZE_OVERLAY_PADDING`)+"金色描边盒子"(仅边框 `{GOLD, width 1.5, radius 10}`、无背景)
  两层叠加,不是单层容器。JSON 条目相应拆成两个子字段:
  ```json
  "maximize_overlay": {
    "scrim": { "background": "SCRIM", "padding": 40 },
    "border": { "color": "GOLD", "width": 1.5, "radius": 10.0 }
  }
  ```
  其中 `Color{0,0,0,0.55}` 目前是游离在 `maximize_overlay` 函数里的原始字面量,不在 14 色令牌
  内。本设计顺带在 `theme.rs` **新增**一个 `pub const SCRIM: Color = ...`(纯增量,不改动"禁止
  改动"的现有 14 个),让这个颜色也能走"引用令牌名"的统一机制,不必在 schema 里为它单开一个
  "允许写原始 rgba"的例外口子。
- **`context_menu_popup`**:菜单外框(背景 `CARD`、边框 `BORDER`、圆角、`padding 6`)之外还有一层
  外部定位用的 `Padding`(用于把菜单摆到右键点击坐标),后者是纯几何定位逻辑而非视觉样式,不纳入
  这份配置——只有菜单外框本身的背景/边框/padding 进 JSON。

其余 10 个区域都是单层容器,严格套用 §3 的通用形状。

## 5. 加载与消费机制

- 新文件:`crates/dozer-app/assets/theme/regions.json`。
- 新模块:`crates/dozer-app/src/chrome_style.rs`:
  - `include_str!("../assets/theme/regions.json")` 编译期内嵌。
  - `std::sync::LazyLock<RegionStyles>`,程序生命周期内解析一次:
    `serde_json::from_str(RAW).expect("regions.json 格式错误")`——解析失败是开发期配置错误,
    直接 panic,不做运行时降级(与"不做运行时读盘"的定位一致:这本质是另一种形式的编译期常量)。
  - `fn resolve_color(name: &str) -> Color`:match 到 `theme::` 里对应常量,未知名字
    `panic!("未知颜色令牌: {name}")`——配置写错在启动时就能发现,不会带着错误的透明色静默跑起来。
  - 每个区域一个访问函数,如 `pub fn top_bar() -> &'static RegionStyle`、
    `pub fn preview_pane() -> &'static RegionStyle`,`maximize_overlay()`/`context_menu_popup()`
    返回各自的复合结构体。
- 调用侧改动:12 个区域函数里现有的
  ```rust
  .style(move |_t: &iced_widget::Theme| container::Style {
      background: Some(theme::PANEL.into()),
      ..container::Style::default()
  })
  ```
  这类内联样式,改成读 `chrome_style::preview_pane()` 拼 `container::Style`(背景/边框从
  `RegionStyle` 取,`.padding(...)`/`.spacing(...)` 调用参数也从同一处取,不再是字面量)。

## 6. 测试与验收方式

- **回归测试**:解析 `regions.json` 后,断言几个关键字段与"改动前"的字面量一致(比如
  `preview_pane` 的 `padding` 应为 `8`、`gap` 应为 `4`),防止以后手滑改错配置文件却没人发现——
  与仓库既有的 `chrome_constants_exclude_removed_header` 这类"锁死防漂移"测试同一个套路。
- **视觉验收**:延续既有约定——编译通过 + 真机跑起来目测,确认改动前后像素级一致(纯代码搬家,
  不应有任何肉眼可见变化)。不引入新的测试哲学。

## 7. 影响范围小结

- 新增:`crates/dozer-app/assets/theme/regions.json`、`crates/dozer-app/src/chrome_style.rs`。
- 修改:`crates/dozer-app/src/theme.rs`(新增 `SCRIM` 常量,不改动现有 14 色)、
  `crates/dozer-app/src/workspace.rs`(12 处区域函数改为读 `chrome_style::` 而非内联字面量)、
  `Cargo.toml`(如 `serde_json` 尚未是 `dozer-app` 的直接依赖,需要添加)。
- 不涉及 `dozerd`/`dozer-core`/`dozer-hook`/`legacy-boy`,纯 `dozer-app` GUI 内部改动。
