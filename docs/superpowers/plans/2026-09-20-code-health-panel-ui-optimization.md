# 代码健康度面板 UI 优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 优化已上线的代码健康度面板（`crates/dozer-app/src/extensions/codehealth/view.rs`）的三处呈现问题：问题列表从"渲染全部函数"收窄成"只展示 Watch/Critical、按严重度+复杂度排序的扁平排行榜"；健康卡片从"只给综合等级"拆成"规模/密度双分档可解释展示"；时间戳从"原始 epoch 数字"改成人类可读格式。

**Architecture:** 纯 UI 呈现层改动，不碰数据层 crate（`dozer-codehealth`）、不碰持久化（`dozerd::code_health`）、不加新 `Message` 变体。三处改动各自独立（互不依赖对方的返回值/签名变化），都落在 `view.rs` 内部函数级别，`content_pane` 对外调用签名不变。每处改动都拆成"纯逻辑函数（可单元测试）+ 渲染函数（人工验收）"两层，逻辑函数用 TDD 覆盖。

**Tech Stack:** Rust, `iced_widget` 0.14（`byteui::theme::color::ColorTokens`），无新增依赖。

**Spec:** `docs/superpowers/specs/2026-09-20-code-health-panel-ui-optimization-design.md`

## Global Constraints

- **在独立分支上开发，不直接提交 main**：`feature/code-health-panel-ui-optimization`，完成后提请审阅，审阅通过后再合并回 main（本仓库既有惯例：两个并行计划直接在 main 上开发曾导致审阅混淆）。
- 只改 `crates/dozer-app/src/extensions/codehealth/view.rs`。**不改** `crates/dozer-codehealth`（`ProjectReport`/`FunctionMetric`/`Severity`/`HealthTier` 字段和算法不变）、**不改** `crates/dozerd/src/code_health.rs`（SQLite schema 不变）、**不改** `codehealth::Message` 枚举（`mod.rs` 预期不需要改动）。
- 颜色只用现有 `byteui::theme::color::ColorTokens` 字段（`red`/`cyan`/`green`/`cream`/`dim`/`body`），不新增 token。**禁止**用 `gold`——CLAUDE.md 明确裁决 `gold` 是甲方动作专属色，本面板是只读展示。
- 不显式调用 `.font(...)`/`.shaping(...)`，沿用 `text()` 默认值（`iced_core::widget::text::Format::default()` 的 `shaping` 字段默认是 `Shaping::Auto`，ASCII 用 Basic、非 ASCII 自动回退 Advanced 做字体 fallback，中文文案已经靠这个默认值正确渲染，不需要手工设置）。
- 不引入 `chrono`/`time` 依赖，时间戳格式化复用仓库已有两份的 Howard Hinnant `civil_from_days` 算法（`crates/dozer-app/src/extensions/git_log.rs::civil_from_days`），本次是第三份"照抄一份"（仓库对这个几行纯函数的既有处理惯例，不为此抽共享 util）。
- 每个任务的逻辑函数变更都要有单元测试；渲染函数（返回 `Element`）的视觉呈现留给最后一个任务的人工验收（`cargo run -p dozer-app`），同仓库既有惯例。

---

### Task 1: 时间戳人类可读格式化

**Files:**
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs:62-67`（`format_ms` 函数体）
- Test: 同文件内新增 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: 无外部依赖（纯 std）。
- Produces: `fn format_ms(ms: u64) -> String`（签名不变，`health_card` 里 `format_ms(ms)` 调用点不需要改）；新增私有 `fn civil_from_days(z: i64) -> (i64, u32, u32)`，仅供 `format_ms` 内部使用，不对外导出。

- [ ] **Step 1: 写失败测试**

在 `view.rs` 文件末尾新增测试模块（如果后续任务也会加测试，这个模块会持续追加，不要重复声明 `mod tests`）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ms_matches_expected_layout() {
        // 2026-09-20 07:58:06 UTC（真实扫描数据的时间戳，固定输入 → 固定输出）。
        assert_eq!(format_ms(1_789_891_086_991), "2026-09-20 07:58:06 UTC");
        // epoch 0 边界。
        assert_eq!(format_ms(0), "1970-01-01 00:00:00 UTC");
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app --lib codehealth::view::tests::format_ms_matches_expected_layout
```

Expected: 编译失败或断言失败（当前 `format_ms` 输出的是 `"{secs}s epoch"` 格式，不匹配新断言）。

- [ ] **Step 3: 实现 `format_ms` 与 `civil_from_days`**

把 `view.rs:62-67` 的 `format_ms` 替换为：

```rust
/// 同 `git_log.rs::format_commit_time` 的处理方式：展示 UTC，不做本地
/// 时区换算——扫描时间戳是纯展示态，UTC 足够，不为此引入时区库。
fn format_ms(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs / 86_400;
    let secs_of_day = secs % 86_400;
    let (h, m, s) = (secs_of_day / 3600, (secs_of_day / 60) % 60, secs_of_day % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

/// Howard Hinnant 的 `civil_from_days` 算法：Unix epoch 起的天数 → (年, 月, 日)。
/// 范围覆盖 1970..=2100。与 `git_log.rs`/`todo.rs` 的同名函数同源，第三份
/// "照抄一份"（那两处已经说明过：跨模块复用一个几行的纯函数不值得引入耦合）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}
```

删除原来那段"面板本身不需要时区感知的复杂格式化"的注释（不再适用——已经做了绝对时间格式化，只是没做时区/相对时间）。

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app --lib codehealth::view::tests::format_ms_matches_expected_layout
```

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/codehealth/view.rs
git commit -m "feat(codehealth): 扫描时间戳改成可读格式(YYYY-MM-DD HH:MM:SS UTC)"
```

---

### Task 2: 健康卡片拆分规模/密度双分档

**Files:**
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs:29-60`（`health_card` 函数）
- Test: 同文件 `#[cfg(test)] mod tests`（追加到 Task 1 建的模块里）

**Interfaces:**
- Consumes: `dozer_codehealth::{ProjectReport, HealthTier}`（已有 import）；`tier_color(tier: HealthTier, tokens: &ColorTokens) -> Color`、`tier_label(tier: HealthTier) -> &'static str`（`view.rs:13-27` 已有，不改）。
- Produces: 新增私有纯函数 `fn density_pct(critical: usize, total: usize) -> f64`、`fn scale_summary(report: &ProjectReport) -> String`、`fn density_summary(report: &ProjectReport) -> String`，供 `health_card` 内部渲染用，也供 Task 4 人工验收时对照文案。`health_card` 签名不变（`fn health_card(report: &ProjectReport, scanned_at_ms: Option<u64>) -> Element<...>`）。

- [ ] **Step 1: 写失败测试**

追加到 `mod tests`：

```rust
    fn sample_report(total_loc: usize, scale_tier: HealthTier, critical_functions: usize, total_functions: usize, density_tier: HealthTier) -> ProjectReport {
        ProjectReport {
            total_loc,
            total_functions,
            critical_functions,
            scale_tier,
            density_tier,
            overall_tier: scale_tier.max(density_tier),
            functions: Vec::new(),
        }
    }

    #[test]
    fn density_pct_computes_percentage() {
        assert!((density_pct(9, 3890) - 0.231_362_47).abs() < 1e-6);
    }

    #[test]
    fn density_pct_zero_total_avoids_div_by_zero() {
        assert_eq!(density_pct(0, 0), 0.0);
    }

    #[test]
    fn scale_summary_formats_loc_and_label() {
        let report = sample_report(101_052, HealthTier::Critical, 9, 3890, HealthTier::Healthy);
        assert_eq!(scale_summary(&report), "规模：警戒（101052 行）");
    }

    #[test]
    fn density_summary_formats_ratio_and_label() {
        let report = sample_report(101_052, HealthTier::Critical, 9, 3890, HealthTier::Healthy);
        assert_eq!(density_summary(&report), "密度：健康（9/3890，约 0.2%）");
    }
```

（`HealthTier` 需要在文件顶部 `use dozer_codehealth::{..., HealthTier};` 已有——检查 `view.rs:8` 现状 import 是否已包含 `HealthTier`，本文件当前只 import 了 `FunctionMetric, HealthTier, ProjectReport, Severity`，已经够用，不需要改 import。）

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app --lib codehealth::view::tests -- density_pct scale_summary density_summary
```

Expected: 编译失败（`density_pct`/`scale_summary`/`density_summary` 尚未定义）。

- [ ] **Step 3: 实现三个纯函数 + 接入 `health_card`**

在 `tier_label` 函数（`view.rs:21-27`）之后插入：

```rust
fn density_pct(critical: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        critical as f64 / total as f64 * 100.0
    }
}

fn scale_summary(report: &ProjectReport) -> String {
    format!(
        "规模：{}（{} 行）",
        tier_label(report.scale_tier),
        report.total_loc
    )
}

fn density_summary(report: &ProjectReport) -> String {
    let pct = density_pct(report.critical_functions, report.total_functions);
    format!(
        "密度：{}（{}/{}，约 {pct:.1}%）",
        tier_label(report.density_tier),
        report.critical_functions,
        report.total_functions
    )
}
```

把 `health_card`（`view.rs:29-60`）里 `row![text(tier_label...), text(summary)...]` 那一行和"上次扫描/扫描按钮"那一行之间，插入一行规模/密度双分档：

```rust
fn health_card(
    report: &ProjectReport,
    scanned_at_ms: Option<u64>,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let badge_color = tier_color(report.overall_tier, &tokens);
    let summary = format!(
        "核心代码 {} 行，{} 个函数存在明显结构问题",
        report.total_loc, report.critical_functions
    );
    let scanned_at = match scanned_at_ms {
        Some(ms) => format!("上次扫描：{}", format_ms(ms)),
        None => "尚未扫描".to_string(),
    };
    column![
        row![
            text(tier_label(report.overall_tier))
                .size(20)
                .color(badge_color),
            text(summary).size(14).color(tokens.body),
        ]
        .spacing(12),
        row![
            text(scale_summary(report))
                .size(12)
                .color(tier_color(report.scale_tier, &tokens)),
            text(density_summary(report))
                .size(12)
                .color(tier_color(report.density_tier, &tokens)),
        ]
        .spacing(12),
        row![
            text(scanned_at).size(12).color(tokens.dim),
            button(text("扫描").size(13)).on_press(Message::ScanRequested),
        ]
        .spacing(12),
    ]
    .spacing(8)
    .padding(16)
    .into()
}
```

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app --lib codehealth::view::tests
```

Expected: PASS（Task 1 + Task 2 的所有测试都过）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/codehealth/view.rs
git commit -m "feat(codehealth): 健康卡片拆分规模/密度双分档展示"
```

---

### Task 3: 问题列表收窄为扁平排行榜

**Files:**
- Modify: `crates/dozer-app/src/extensions/codehealth/view.rs:69-107`（`problem_row` + `problem_list` 函数）
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `dozer_codehealth::{FunctionMetric, Severity}`（已有 import）。
- Produces: 新增私有 `fn severity_rank(s: Severity) -> u8`、`fn ranked_problems(report: &ProjectReport) -> Vec<&FunctionMetric>`。`problem_row`/`problem_list` 签名不变（`problem_list(report: &ProjectReport) -> Element<...>`），`content_pane` 里 `problem_list(report)` 调用点不需要改。

- [ ] **Step 1: 写失败测试**

追加到 `mod tests`：

```rust
    fn metric(name: &str, severity: Severity, complexity_signal: usize) -> FunctionMetric {
        FunctionMetric {
            name: name.to_string(),
            file: std::path::PathBuf::from("a.rs"),
            start_line: 1,
            end_line: 2,
            loc: 2,
            complexity_signal,
            severity,
        }
    }

    fn report_with_functions(functions: Vec<FunctionMetric>) -> ProjectReport {
        ProjectReport {
            total_loc: 0,
            total_functions: functions.len(),
            critical_functions: functions.iter().filter(|f| f.severity == Severity::Critical).count(),
            scale_tier: HealthTier::Healthy,
            density_tier: HealthTier::Healthy,
            overall_tier: HealthTier::Healthy,
            functions,
        }
    }

    #[test]
    fn severity_rank_orders_critical_highest() {
        assert!(severity_rank(Severity::Critical) > severity_rank(Severity::Watch));
        assert!(severity_rank(Severity::Watch) > severity_rank(Severity::Normal));
    }

    #[test]
    fn ranked_problems_filters_out_normal() {
        let report = report_with_functions(vec![
            metric("a", Severity::Normal, 5),
            metric("b", Severity::Watch, 20),
        ]);
        let ranked = ranked_problems(&report);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].name, "b");
    }

    #[test]
    fn ranked_problems_sorts_by_severity_then_complexity_desc() {
        let report = report_with_functions(vec![
            metric("watch_low", Severity::Watch, 16),
            metric("critical_low", Severity::Critical, 41),
            metric("critical_high", Severity::Critical, 255),
            metric("watch_high", Severity::Watch, 40),
        ]);
        let ranked = ranked_problems(&report);
        let names: Vec<&str> = ranked.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["critical_high", "critical_low", "watch_high", "watch_low"]
        );
    }

    #[test]
    fn ranked_problems_empty_when_all_normal() {
        let report = report_with_functions(vec![metric("a", Severity::Normal, 0)]);
        assert!(ranked_problems(&report).is_empty());
    }
```

（`HealthTier` 同 Task 2，已经在文件顶部 import 里。）

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozer-app --lib codehealth::view::tests -- severity_rank ranked_problems
```

Expected: 编译失败（`severity_rank`/`ranked_problems` 尚未定义）。

- [ ] **Step 3: 实现 `severity_rank` + `ranked_problems`，重写 `problem_row`/`problem_list`**

把 `view.rs:69-107` 的 `problem_row` + `problem_list` 整段替换为：

```rust
fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Critical => 2,
        Severity::Watch => 1,
        Severity::Normal => 0,
    }
}

/// 过滤掉 `Severity::Normal`、按严重度降序 + 复杂度降序排出扁平排行榜。
/// `report.functions` 本身已经按严重度排过（`dozer-codehealth::report::scan_project`
/// 的契约，供 dozer-mcp 未来复用全量数据，这里不改那份排序，只在展示层重排一份
/// 过滤后的视图）。`severity_rank` 是本文件私有的排名映射，不是
/// `dozer_codehealth::FunctionMetric` 的 `pub(crate)` 方法（那个跨 crate 不可见）。
fn ranked_problems(report: &ProjectReport) -> Vec<&FunctionMetric> {
    let mut v: Vec<&FunctionMetric> = report
        .functions
        .iter()
        .filter(|f| f.severity != Severity::Normal)
        .collect();
    v.sort_by_key(|f| {
        (
            std::cmp::Reverse(severity_rank(f.severity)),
            std::cmp::Reverse(f.complexity_signal),
        )
    });
    v
}

fn problem_row(
    f: &FunctionMetric,
    tokens: &ColorTokens,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
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
                .size(12)
                .color(color),
        ]
        .spacing(8),
        text(f.file.display().to_string()).size(11).color(tokens.dim),
    ]
    .spacing(2)
    .padding([4, 8]);
    mouse_area(content)
        .on_press(Message::OpenLocation(f.file.clone(), f.start_line))
        .into()
}

fn problem_list(
    report: &ProjectReport,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let ranked = ranked_problems(report);
    if ranked.is_empty() {
        return container(
            text("没有发现结构复杂的函数，代码整体健康。")
                .size(13)
                .color(tokens.dim),
        )
        .padding(16)
        .into();
    }
    let mut col = Column::new().spacing(4);
    for f in ranked {
        col = col.push(problem_row(f, &tokens));
    }
    scrollable(col.padding(16)).into()
}
```

删除文件顶部 `use std::collections::BTreeMap;`（`view.rs:11`）——`BTreeMap` 分组不再需要，删掉避免 `cargo clippy` 的未使用 import 警告。

- [ ] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozer-app --lib codehealth::view::tests
```

Expected: PASS（Task 1/2/3 的所有测试都过）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/codehealth/view.rs
git commit -m "feat(codehealth): 问题列表收窄成 Watch/Critical 扁平排行榜"
```

---

### Task 4: 整体编译检查 + 人工验收

**Files:** 无新改动，只验证 Task 1-3 的组合结果。

**Interfaces:** 无（验收任务）。

- [ ] **Step 1: 全量测试**

```bash
cargo test -p dozer-app --lib codehealth::view::tests
```

Expected: PASS，Task 1/2/3 全部测试通过（`format_ms_matches_expected_layout`、`density_pct_*`、`scale_summary_*`、`density_summary_*`、`severity_rank_*`、`ranked_problems_*` 共 10 个测试）。

- [ ] **Step 2: Clippy + fmt 检查**

```bash
cargo clippy -p dozer-app --all-targets -- -D warnings
cargo fmt --check
```

Expected: 无警告、无格式差异。若 `cargo fmt` 有差异，运行 `cargo fmt`（不带 `--check`）后重新 `git add` 并追加一个 `style: cargo fmt` commit。

- [ ] **Step 3: 人工验收——启动 GUI 核对三处改动**

```bash
cargo run -p dozer-app
```

打开代码健康度面板（Rail 右侧图标栏），核对：

1. 健康卡片：综合徽章下方出现"规模：警戒（101052 行） 密度：健康（9/3890，约 0.2%）"这一行（数值以当前项目实际扫描结果为准，若还没扫描过先点"扫描"按钮）。
2. 问题列表：不再按文件路径分组，是一个扁平列表，每行前面有"警戒"/"关注"文字标签（不再只靠颜色区分），最上面一条应该是 `complexity` 值最高的函数（不管它在哪个文件），文件路径以次要小字显示在函数名下方。列表条数应该明显少于全部函数数（只剩 Watch+Critical）。
3. 点击某一行，确认仍能跳转到 Files 面板对应代码行（`Message::OpenLocation` 链路本次没改，但要确认没有被 Step 3 的改动意外破坏）。
4. 时间戳："上次扫描：" 后面是 `YYYY-MM-DD HH:MM:SS UTC` 格式，不再是 `NNNNNNNNNNs epoch`。
5. （如果当前项目恰好没有任何 Watch/Critical 函数）确认问题列表区域显示"没有发现结构复杂的函数，代码整体健康。"，不是空白一片。

- [ ] **Step 4: Commit（若 Step 2 产生了 fmt 差异）**

```bash
git add crates/dozer-app/src/extensions/codehealth/view.rs
git commit -m "style(codehealth): cargo fmt"
```

（若 Step 2 无差异，跳过本步骤，不产生空 commit。）
