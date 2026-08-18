# byteui:跨产品复用的 UI 组件库设计

**状态:已批准(brainstorming 会话,2026-08-18)**

## 背景

Dozer 现在的 UI 抽象散落在 `crates/dozer-app/src/` 里:`icons.rs`(图标渲染 +
`icon_button_entry` 交互内核)、`tabs.rs`(`tab_core` 交互内核)、
`theme/cards.rs`(卡片三态样式)、`scrollbar.rs`(统一滚动条)、
`theme/{color,font,geometry,icon_size,region}.rs`(设计 token,编译期内嵌
`assets/theme/workspace.json`)。这套东西是 2026-08-12 那轮"消除重复"重构
(见 `2026-08-12-tab-icon-button-shared-components-design.md`)之后逐步长出来
的,已经证明"交互内核(只管行为)+ 样式函数(只管外观)"这个分层在本项目里
好用,但它天生长在 `dozer-app` crate 内部,和这个 crate 的其他业务代码
(`app.rs`/各 `extensions/*`)没有边界。

ByteBoy 后续会有第二个、第三个 iced/Rust 桌面产品,都需要同一套视觉语言
(甲方向治理工具的暗色主题、卡片/tab/图标按钮这些基础交互)。现在的写法
意味着每个新产品要么整体复制 `dozer-app` 这几个文件,要么从头重写——两条
路都会导致视觉和交互细节逐渐漂移,重蹈 2026-08-12 那次"同一个概念两处实现
各自长歪"的覆辙,只是这次漂移发生在产品之间而不是文件之间。

这次设计把这套已验证的模式提炼成独立 crate `byteui`,组件命名对齐
[amis](https://baidu.github.io/amis/zh-CN/components) 的分类和组件名(已按
`baidu/amis` 仓库 `docs/zh-CN/components/` 源码目录核对,不是凭印象),这样
组件的语义/职责边界可以直接查 amis 文档理解,不用另写一份等价说明。

**不做 amis 那种 JSON schema 驱动 UI 的能力**(brainstorming 会话中已讨论并
排除)——那解决的是"非工程师用可视化编辑器拼页面"的问题,需要表达式求值/
数据绑定/动作系统一整套运行时,而 Dozer 的使用者是直接写 Rust 的开发者。
`byteui` 只借 amis 的**命名和分类**,组件树继续在 Rust 代码里写死,唯一
"配置驱动"的部分和现状一致:`workspace.json` 驱动的设计 token(颜色/字号/
间距/图标尺寸),这个粒度不变。

## 目标 / 非目标

**目标**:

1. 新建 `crates/byteui`,把设计 token 系统(`theme/*`)和已验证的交互内核
   (`icon_button_entry`/`tab_core`/`cards`/`scrollbar`)原样迁入,作为库的
   起点——这部分是"平移",不是重新设计。
2. 在此基础上补齐 amis 分类里 Dozer 当前用得到、但现在只有各面板临时手拼
   实现的组件(见下方"v1 组件清单")。
3. Theme 做成可替换的运行时全局单例,`ByteBoy2077` 是编译期默认值;未来
   产品只需提供一份不同的 token 取值,组件代码不用改。
4. `byteui` 不依赖 `dozer-core`,不内置任何 Dozer 专属的路径/业务约定
   ——这是唯一一处需要主动从现状"拔出来"的改动(见"依赖边界"一节)。

**非目标**:

- **不做 JSON schema 驱动 UI**(已在背景一节说明,brainstorming 会话中
  明确排除)。
- **不做完整的 amis Table**(排序/虚拟滚动/列定义的重型表格组件)。Dozer
  现在没有一处真正需要"表格"语义的地方(Usage/git log 都是自定义列表/
  Canvas),先不为假设的未来买单。
- **不在这次把 `dozer-app` 迁移过去**。新库先独立建好结构、跑通编译和
  单测,dozer-app 现有的 `icons.rs`/`tabs.rs`/`theme/*` 暂时原样保留;
  迁移是后续增量任务,一个调用点一个调用点切,不是这次 spec 的范围
  (参考 2026-08-12 那次"落地共享组件"和"迁移调用点"本就是分两步走)。
- **不做 Badge(角标)和 Tag(分类标签)**。amis 里两者都存在但 Dozer
  现在没有明确用途,不进 v1。
- **不引入 HBox**。amis 的 `HBox`(比例分栏)和 `Flex`(css flex 封装)
  功能有重叠,iced 的 `row!`/`column!` 本身就是 flex 模型
  (`Length::FillPortion` 对应 grow),只保留语义最贴近底层的 `Flex`。
- **不给 `byteui` 引入新的 GUI 框架依赖或渲染后端**。仍然是
  `iced_widget`/`iced_renderer` 0.14 系,和 `dozer-app` 当前版本对齐。

## 架构与数据流

### crate 结构

```
crates/byteui/
├── Cargo.toml              # iced_widget/iced_renderer/serde/serde_json,无 dozer-core
├── assets/theme/
│   └── workspace.json      # 从 dozer-app 原样迁入,后续两边各自维护自己的取值
└── src/
    ├── lib.rs
    ├── theme/               # 原样迁入 dozer-app 的 theme/{color,font,geometry,icon_size,region}.rs
    │   ├── mod.rs
    │   ├── color.rs         #  运行时可替换,ByteBoy2077 是编译期默认值(见下)
    │   ├── font.rs
    │   ├── geometry.rs
    │   ├── icon_size.rs      # persist_scale/load_persisted_scale 改为吃 &Path 参数
    │   └── region.rs
    ├── interaction/         # 原样迁入,只改 crate 内部引用路径
    │   ├── icons.rs          # IconKind + icon_button_entry + with_tooltip
    │   ├── tabs.rs            # tab_core
    │   ├── cards.rs           # button_card/container_card
    │   └── scrollbar.rs
    ├── layout/               # 新增
    │   ├── divider.rs
    │   ├── panel.rs
    │   ├── wrapper.rs
    │   └── flex.rs
    ├── form/                 # 新增
    │   ├── input_text.rs
    │   ├── select.rs
    │   ├── checkbox.rs
    │   └── switch.rs
    ├── feedback/              # 新增
    │   ├── status.rs
    │   ├── progress.rs
    │   └── toast.rs           # ToastQueue(纯数据)+ view(&ToastQueue)
    └── data/                  # 新增
        ├── card.rs             # 复用 interaction::cards 的三态样式
        ├── list.rs
        └── property.rs
```

`interaction/` 这个模块名是新起的——现状里 `icons.rs`/`tabs.rs`/`cards.rs`
是平级散在 `dozer-app/src/` 下,搬进 `byteui` 后归到同一个模块下,和
`layout`/`form`/`feedback`/`data` 四个 amis 风格分类平级,反映它们的共同
职责("交互内核",不是具体的可视化组件分类)。

### Theme:运行时可替换的全局单例

延续 `icon_size::scale()` 已经验证过的模式(启动读 JSON 默认值,运行时可用
`AtomicU32`/等价机制改写,不侵入每个函数签名多传一个 `&Theme` 参数)。这个
选择相对"显式传参注入 Theme"的替代方案,代价是全局可变状态,但和现有
`dozer-app` 代码库的既有习惯(`theme::color::GOLD` 之类的全局 const/
`theme::icon_size::rail()` 之类读全局的 accessor)对齐,未来迁移调用点时
改动量最小——这是 brainstorming 会话里明确权衡过的取舍。

```rust
// byteui::theme::color 之类模块内部结构示意:
pub struct Theme {
    pub color: ColorTokens,   // BG/CARD/BORDER/CREAM/GOLD/CYAN/GREEN/...
    pub font: FontTokens,
    pub geometry: GeometryTokens,
    pub icon_size: IconSizeTokens,
}

static CURRENT: RwLock<Theme> = ...; // 编译期默认值 = ByteBoy2077(从 workspace.json 解析)

pub fn current() -> Theme { CURRENT.read().unwrap().clone() }
pub fn set_theme(t: Theme) { *CURRENT.write().unwrap() = t; }
```

组件内部一律调 `theme::current()` 取值,不再有硬编码色值/字号常量——这是
和现状(`icons.rs`/`tabs.rs` 直接引用 `crate::theme::color::GOLD` 这样的
const)唯一的语义差异:换主题 = 换一份 `Theme` 值整体替换掉这个全局单例,
组件代码不用碰。

### v1 组件清单(amis 命名对照)

| 分类 | amis 组件(docs 文件) | `byteui` 路径 | 说明 |
|---|---|---|---|
| 布局 | `divider.md` | `layout::divider` | 分隔线,纯样式函数 |
| 布局 | `panel.md` | `layout::panel` | 带描边圆角的容器,取代现有各面板临时拼的 `container` 样式 |
| 布局 | `wrapper.md` | `layout::wrapper` | 无装饰的间距包裹(padding 语义化) |
| 布局 | `flex.md` | `layout::flex` | 水平/垂直排列 + gap token,取代散落各处的裸 `row!.spacing(N)` |
| 表单 | `form/input-text.md` | `form::input_text` | 单行文本输入,统一边框/focus 态(Todo 新增任务框、SSH 表单目前各自手写) |
| 表单 | `form/select.md` | `form::select` | 下拉选择 |
| 表单 | `form/checkbox.md` | `form::checkbox` | 复选框 |
| 表单 | `form/switch.md` | `form::switch` | 开关 |
| 反馈 | `status.md` | `feedback::status` | 成功/失败/进行中状态展示(Todo/验收面板现在各自拼文字+颜色) |
| 反馈 | `progress.md` | `feedback::progress` | 进度条 |
| 反馈 | `toast.md` | `feedback::toast` | 轻提示,状态归属见下 |
| 数据展示 | `card.md` | `data::card` | 复用 `interaction::cards` 三态样式,加内容布局约定 |
| 数据展示 | `list.md` | `data::list` | 列表项行(取代 Agent/项目/最近/git commit 等列表各自手拼) |
| 数据展示 | `property.md` | `data::property` | key-value 网格展示,对应项目信息面板/Usage 面板"字段:值"这类信息 |

**Rust 命名规则**:模块/函数用 amis 组件的 kebab-case type 名转 snake_case
(`input-text` → `input_text`),不额外发明命名——这是这次设计明确要达成的
效果:组件是什么、大致职责是什么,直接查 amis 对应页面就有说明,不用
`byteui` 自己再写一份等价文档。

### Toast 的状态归属

`feedback::toast` 是 v1 里唯一需要跨帧存活状态的组件,和其他纯视图函数
不同,处理方式:

- `byteui` 只提供 `ToastQueue`(纯数据结构,不碰 iced 事件循环):
  `push(toast)`、`retain_active(now: Instant)`(清掉过期项)。
- `feedback::toast::view(&ToastQueue) -> Element` 是纯渲染函数,吃当前
  队列快照,吐出叠层用的 `Element`。
- 队列本身由消费方(如 `dozer-app` 的 `State`)持有并驱动:
  在已有的动画 tick 循环里顺带调 `queue.retain_active(Instant::now())`,
  在需要弹提示的地方调 `queue.push(...)`,自己决定把 `toast::view` 结果
  叠在哪一层 `stack!` 里。

这样 `byteui` 整体保持"无状态纯函数集合"的定位,和 `icon_button_entry`/
`tab_core` 的既有风格一致——`ToastQueue` 是极少数例外(state 本身),但
它不持有任何 iced 类型,不产生副作用,仍然是纯数据结构。

### 依赖边界:不依赖 `dozer-core`

现状 `theme/icon_size.rs` 用 `dozer_core::paths::config_dir()` 存运行时
缩放值——这是"Dozer 专属路径约定"混进了本该通用的组件库代码。迁入
`byteui` 时改成:

```rust
// 现状(dozer-app 内):
fn scale_path() -> PathBuf { dozer_core::paths::config_dir().join("ui_scale.json") }

// byteui 内:
pub fn persist_scale(path: &Path) { save_to(path, scale()) }
pub fn init_scale(path: &Path) { if let Some(v) = load_from(path) { set_scale(v) } }
```

调用方(`dozer-app`)自己算好路径传进来。这是这次设计里唯一一处要求
"主动改现有实现细节"而不是原样平移的地方,原因是这个函数一旦留在库里
硬编码 `dozer_core`,后续 ByteBoy 产品复用时就得连 `dozer-core` 一起拉进来
——而 `dozer-core` 是 Dozer 自己的会话协议/路径约定 crate,对其他产品没
意义。

## 错误处理

沿用现有 `theme/icon_size.rs` 的策略:`workspace.json` 解析失败直接
`panic`(开发期配置错误,不是运行时需要优雅降级的数据),和 `dozer-hook`
"运行时错误静默"的定位刻意不同——这里是构建/打包期就该发现的问题。

持久化的运行时状态(缩放值等)遵循相反的策略:读取失败(文件缺失/损坏/
越界)一律回落默认值,不 panic——这部分数据来自用户历史操作,不是开发期
配置。两条策略都是现状(`icon_size.rs`)已经在用的,原样保留。

## 测试策略

只覆盖纯逻辑,不做 widget 渲染快照测试(workspace 里没有这类测试的先例,
`icons.rs`/`icon_size.rs` 现有测试也都是纯逻辑):

1. `theme` 模块:token 解析(`tokens_match_pre_migration_literals` 这类
   防漂移锚测试原样迁移)、scale 落盘 round-trip、损坏文件回落默认值。
2. `feedback::toast`:`ToastQueue::retain_active` 的过期裁剪逻辑(边界:
   恰好到期、多条同时到期、空队列)。
3. `interaction::icons`:`icon_for_file` 扩展名映射(原样迁移现有测试)。
4. `cargo build -p byteui` 独立编译通过(不依赖 `dozer-app`/`dozer-core`
   即可编译,是对"依赖边界"设计的直接验证)。

## 排期备注

这次 spec 只覆盖 `byteui` 本身的建库与 v1 组件——`dozer-app` 侧的迁移
是后续独立任务,一个模块一个调用点地做,不要求一次性切完。建库本身可以
作为一个完整的实现计划一次做完(内部各模块相互独立,`layout`/`form`/
`feedback`/`data` 四类之间没有依赖关系,可以并行或任意顺序实现),`theme`/
`interaction` 两块是平移,应最先做,后面几类新组件都会用到里面的
token/卡片样式。
