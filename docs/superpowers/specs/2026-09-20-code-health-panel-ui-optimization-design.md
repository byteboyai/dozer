# 代码健康度面板 UI 优化设计

**状态：已批准（brainstorming 会话，2026-09-20）**

**关联 spec**：`docs/superpowers/specs/2026-09-20-code-health-panel-design.md`（一期设计，
下称"v1 spec"）。本文档是 v1 上线后基于真实扫描数据发现的 UI 呈现层优化，**不**重新
定义指标算法、不改数据层 crate、不改产品定位（"预警 + 定位，不做解决"），v1 spec 的
所有裁决继续有效。

## 背景

Dozer 项目自身首次跑完代码健康度扫描后（`total_loc=101052`、`total_functions=3890`、
`critical_functions=9`、`overall_tier=Critical`），对着真实数据复盘发现当前 UI
（`crates/dozer-app/src/extensions/codehealth/view.rs`）呈现方式有三处具体问题：

1. **问题列表没有过滤**：`problem_list()` 遍历 `report.functions` 的全部 3890 个函数
   （按文件路径分组，只用颜色区分严重度），而不是只展示真正需要关注的 29 个
   （9 Critical + 20 Watch）。面向 v1 spec 明确的"非职业程序员"受众，一次性铺出近
   4000 行、其中 99.3% 都是"没事"的噪音，直接违背"预警"该有的信噪比。
2. **综合等级不可解释成因**：健康卡片只显示 `overall_tier` 一个徽章 +
   一句摘要（"核心代码 101052 行，9 个函数存在明显结构问题"），没有说明这次
   `Critical` 是 `scale_tier`（规模，101052 行 > 2 万行阈值）触发的，而
   `density_tier`（密度，9/3890 ≈ 0.2%）其实是 `Healthy`。v1 spec 在"项目级"一节
   明确强调"可解释性优先：为什么是这个等级要能一句话讲清楚给非程序员朋友听"，当前
   UI 没有兑现这一点。
3. **时间戳是原始 epoch 数字**："上次扫描：1789891086s epoch"——v1 spec 里
   `format_ms` 当时明确写的是 YAGNI 占位（"精确格式化不是本设计的核心诉求"），现在
   顺手补上。

## 范围

**纯 UI 呈现层改动**，只改：

- `crates/dozer-app/src/extensions/codehealth/view.rs`（主要改动：健康卡片、问题列表
  过滤排序、时间戳格式化）
- `crates/dozer-app/src/extensions/codehealth/mod.rs`（如果需要，仅限
  `WorkspaceState`/`update` 不变的前提下调整；预期不需要改动）

**不改**：

- `crates/dozer-codehealth`（`ProjectReport`/`FunctionMetric`/`Severity`/`HealthTier`
  的字段、算法、阈值全部不变——`report.functions` 继续是"全部函数按严重度降序"的
  通用契约，供 v1 spec"未来方向"第 7 条里设想的 `dozer-mcp` 复用留着，过滤/重排是
  纯展示态决策，不下沉进数据层）。
- `crates/dozerd/src/code_health.rs`（SQLite 落盘 schema 不变）。
- `codehealth::Message` 枚举（不加新变体，`ScanRequested`/`Scanned`/`Loaded`/
  `OpenLocation` 均不变，交互流程不变）。
- 依赖（不引入 `chrono`/`time`，见下方"时间戳格式化"）。

## UI 设计

### 1. 健康卡片：拆分规模 / 密度双分档

现状 `health_card()` 只画一个 `overall_tier` 徽章 + 一句摘要。改造后在摘要下方加一行，
分别用 `tier_label`/`tier_color`（复用现有 helper，不新增）渲染 `scale_tier` 与
`density_tier`：

```
[警戒]  核心代码 101052 行，9 个函数存在明显结构问题
规模：警戒 —— 代码量已经很大（101052 行）      密度：健康 —— 问题函数占比很低（9/3890，约 0.2%）
上次扫描：2026-09-19 08:38:06 UTC              [扫描]
```

```rust
fn tier_breakdown_row(report: &ProjectReport, tokens: &ColorTokens) -> Element<'_, ...> {
    let density_pct = if report.total_functions == 0 {
        0.0
    } else {
        report.critical_functions as f64 / report.total_functions as f64 * 100.0
    };
    row![
        text(format!("规模：{}", tier_label(report.scale_tier)))
            .size(12).color(tier_color(report.scale_tier, tokens)),
        text(format!("（{} 行）", report.total_loc)).size(12).color(tokens.dim),
        text(format!("密度：{}", tier_label(report.density_tier)))
            .size(12).color(tier_color(report.density_tier, tokens)),
        text(format!("（{}/{}，约 {density_pct:.1}%）", report.critical_functions, report.total_functions))
            .size(12).color(tokens.dim),
    ].spacing(12).into()
}
```

插入 `health_card()` 的 `column![...]` 中，摘要行和"上次扫描/扫描按钮"行之间。

文案措辞（实现时可微调用词，但要传达"规模、密度是两回事"）：不用"阈值"这类术语，
用"代码量已经很大"/"问题函数占比很低"这种大白话，延续 v1 spec 摘要句的风格。

### 2. 问题列表：过滤 + 扁平排行榜

现状 `problem_list()`：`BTreeMap<&Path, Vec<&FunctionMetric>>` 按文件路径字母序分组，
组内保留 `report.functions` 原有的按严重度降序，文件路径是分组标题。

改造：

```rust
fn ranked_problems(report: &ProjectReport) -> Vec<&FunctionMetric> {
    let mut v: Vec<&FunctionMetric> = report
        .functions
        .iter()
        .filter(|f| f.severity != Severity::Normal)
        .collect();
    // report.functions 已按 severity 降序，这里补一个 complexity_signal 降序
    // 的次级排序键，让同一严重度内也按"越复杂越靠前"排——扁平排行榜而不是
    // 按文件分组，`severity_rank` 是 dozer-codehealth 内部 pub(crate) 字段，
    // 跨 crate 不可见，这里本地重建一份排名映射（同 git_log.rs/todo.rs 里
    // `civil_from_days` 各处照抄一份小函数的既有惯例，不为此改 dozer-codehealth
    // 的可见性）。
    v.sort_by_key(|f| (std::cmp::Reverse(severity_rank(f.severity)), std::cmp::Reverse(f.complexity_signal)));
    v
}

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Critical => 2,
        Severity::Watch => 1,
        Severity::Normal => 0,
    }
}
```

行渲染：文字标签（复用健康卡片同一套"警戒"/"关注"词汇，不单靠颜色——面向非程序员
用户，色盲/弱视也要能读懂）+ 函数名 + `complexity`/`loc` + 文件路径降级为行内次要信息
（不再是分组标题）：

```rust
fn problem_row(f: &FunctionMetric, tokens: &ColorTokens) -> Element<'_, ...> {
    let (label, color) = match f.severity {
        Severity::Critical => ("警戒", tokens.red),
        Severity::Watch => ("关注", tokens.cyan),
        Severity::Normal => unreachable!("ranked_problems 已过滤掉 Normal"),
    };
    let content = column![
        row![
            text(label).size(11).color(color),
            text(&f.name).size(13).color(tokens.cream),
            text(format!("complexity={} loc={}", f.complexity_signal, f.loc))
                .size(12).color(color),
        ].spacing(8),
        text(f.file.display().to_string()).size(11).color(tokens.dim),
    ].spacing(2).padding([4, 8]);
    mouse_area(content)
        .on_press(Message::OpenLocation(f.file.clone(), f.start_line))
        .into()
}

fn problem_list(report: &ProjectReport) -> Element<'_, ...> {
    let tokens = byteui::theme::color::current();
    let ranked = ranked_problems(report);
    if ranked.is_empty() {
        return container(
            text("没有发现结构复杂的函数，代码整体健康。")
                .size(13).color(tokens.dim),
        ).padding(16).into();
    }
    let mut col = Column::new().spacing(4);
    for f in ranked {
        col = col.push(problem_row(f, &tokens));
    }
    scrollable(col.padding(16)).into()
}
```

**空态**是本次新引入的边界情况：v1 spec 的空态只覆盖"项目下没有 `.rs` 文件"（整个
`report` 是 `None`）；过滤之后，`report` 存在但 `ranked_problems` 为空（项目很健康，
没有任何 Watch/Critical 函数）是一种新的、之前不会出现的展示态，需要单独提示，不能
留白。

**明确排除**（本次讨论中问清楚的非目标）：

- **不加"展开显示全部函数"开关**。彻底只显示问题函数；想看全量数据的人（目前只有
  开发者自己）可以直接查 SQLite 落盘结果，或等 v1 spec"未来方向"第 7 条
  `dozer-mcp` 查询能力落地，不属于本面板职责。
- **不加文件级摘要行**（如"N 个文件需要关注"）。排行榜本身已经把每个问题函数所在
  文件当作行内次要信息展示了，再叠一层文件汇总（对应 `dozer-codehealth::FileMetric`，
  数据层已有但 UI 从未消费）会让卡片和"规模/密度"两个分档信息竞争注意力，YAGNI。
- **不做虚拟化/分页**。过滤到只剩 Watch+Critical 后列表规模通常是几十行量级（本次
  真实数据 29 行），`scrollable` 直接渲染即可，原本"一次性渲染近 4000 个 widget"的
  潜在性能顾虑随着过滤天然解决，不需要额外的列表虚拟化机制。

### 3. 时间戳格式化

现状 `format_ms(ms: u64) -> String` 输出 `"{secs}s epoch"`。改造为
`YYYY-MM-DD HH:MM:SS UTC`，复用仓库里已经出现过两次的 Howard Hinnant
`civil_from_days` 民用历算法（`crates/dozer-app/src/extensions/git_log.rs::civil_from_days`、
`crates/dozer-app/src/extensions/todo.rs` 同名私有函数），**不引入 `chrono`/`time`
依赖**，纯 std：

```rust
/// 同 git_log.rs::format_commit_time 的处理方式：展示 UTC，不做本地时区
/// 换算（扫描时间戳是纯展示态，UTC 足够，不为此引入时区库）。
fn format_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs / 86_400;
    let secs_of_day = secs % 86_400;
    let (h, m, s) = (secs_of_day / 3600, (secs_of_day / 60) % 60, secs_of_day % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

/// 与 git_log.rs/todo.rs 同名函数同源，第三份"照抄一份"（那两处的注释已经
/// 说明了为什么不抽共享 util：跨模块复用一个几行的纯函数不值得引入耦合）。
fn civil_from_days(z: i64) -> (i64, u32, u32) { /* 同 git_log.rs 实现 */ }
```

不做相对时间（"3 小时前"）——v1 spec 当时的 YAGNI 判断（"精确格式化不是本设计的核心
诉求"）在绝对时间戳这个粒度上依然成立，本次只解决"完全不可读"的问题，不过度设计。

## UI token / 组件一致性

（用户明确要求：改动要跟当前项目 UI token 和 UI 库保持一致，逐条核实如下）

- **颜色**：只用 `byteui::theme::color::ColorTokens` 已有字段
  （`red`/`cyan`/`green`/`cream`/`dim`/`body`），与 `health_card`/`problem_row` 现状
  完全一致，不新增 token。不使用 `gold`——CLAUDE.md 明确 `gold` 是甲方动作专属色，
  本面板是只读展示（"预警 + 定位，不做解决"），不产生甲方动作。
- **字体**：不显式调用 `.font(...)`，沿用 `text()` 默认。已读 iced 0.14 源码
  （`iced_core::widget::text::Format::default()`）确认 `text()` 的 `shaping` 字段
  默认是 `Shaping::Auto`（ASCII 用 Basic、非 ASCII 自动回退到 Advanced 做字体
  fallback），当前面板已有的中文摘要文案（"核心代码...行..."）正是靠这个默认值正常
  渲染的，本次新增的中文文案（"规模：警戒"、"警戒"/"关注"标签、空态提示）走同一条
  路径，不需要额外设置。CLAUDE.md"非 ASCII 文本靠 Shaping::Advanced 做字体回退"
  的裁决在这里已经被 `Auto` 默认值满足，不需要手工干预。
- **字号**：延续现状层级——正文 12~14px（`health_card` 摘要 14、`problem_row`
  函数名 13）、次要信息 11~12px（文件路径、时间戳），本次新增的"规模/密度"行、
  严重度文字标签、空态提示均落在这个既有层级里（12/13/11），不引入新字号。
- **组件**：不引入新组件库或自绘控件。`row`/`column`/`text`/`mouse_area`/
  `scrollable`/`button`/`container` 全部沿用当前文件已导入的 `iced_widget`，
  和 `files`/`todo` 等其余 extension 是同一套基础组件，不新增依赖。
- **不涉及**icon 按钮、tab 类 UI，因此不触发 CLAUDE.md"新增/改造 icon 按钮、tab
  类 UI 优先复用 `icons::icon_button_entry`/`tabs::tab_core`"那条裁决——本次没有
  新增这两类控件。

## 测试策略

- `ranked_problems`：单元测试覆盖过滤（`Normal` 不出现在结果里）、排序（构造混合
  severity/complexity 的 fixture，验证 Critical 全部排在 Watch 前面，同 severity 内
  按 complexity 降序）、空结果（全部函数都是 `Normal` 时返回空 `Vec`）。
- `format_ms`：复用 `git_log.rs::format_commit_time_matches_expected_layout` 同款
  测试模式，验证已知 `ms` 值格式化出的字符串符合 `YYYY-MM-DD HH:MM:SS UTC` 布局。
- `tier_breakdown_row` 的密度百分比计算：覆盖 `total_functions == 0`（避免除零，
  显示 0.0%）。
- 卡片配色、排行榜视觉呈现、空态提示留给 `cargo run -p dozer-app` 人工验收，同
  v1 spec 既有惯例（面板渲染类改动这个仓库一贯不写像素级快照测试）。

## 未来方向（明确不在本次范围）

延续 v1 spec 的"记录不代表排期"原则：

- **展开查看全量函数数据**：留给 `dozer-mcp` 查询能力（v1 spec"未来方向"第 7 条）
  或直接查 SQLite，不在本面板加交互。
- **文件级摘要**：`FileMetric` 数据已有，UI 呈现需求出现时再单独立项，不跟本次
  排行榜/双分档改动捆绑。
