# byteui 独立仓库化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer/crates/byteui` 拆成独立仓库 `byteboyai/byteui`（带完整提交历史），让 dozer 与 digger 通过 git 依赖共用它，并补上 digger 设计稿需要的 Badge / Tag / Button。

**Architecture:** 先在 dozer 仓库内把 byteui 清理到"不含任何 dozer 专属内容"（注释、基线测试、环境变量名），再用 `git filter-repo` 导出历史建新仓库并发 `v0.1.0`（内容与清理后的现状一致），然后 dozer 改为 git 依赖并删除本地 crate，最后在新仓库里加三个组件与组件展示示例，发 `v0.2.0`。dozer-app 的调用点全程不改。

**Tech Stack:** Rust 2024，iced 0.14（`iced_widget`/`iced_renderer`），`git filter-repo`，GitHub Actions，`gh` CLI。

**Spec:** `docs/superpowers/specs/2026-10-01-byteui-standalone-repo-design.md`

## Global Constraints

- **dozer 侧的工作必须在独立 worktree 分支里做，不在 main 上直接提交**：`git worktree add .worktrees/byteui-standalone -b feat/byteui-standalone`（`.worktrees/` 已有先例）。dozer 主工作区目前有别人未提交的改动（`Cargo.lock`、`crates/dozer-app/Cargo.toml` 等），不要碰、不要 stash 掉。每次 `git add`/`git commit` 前先 `git branch --show-current` 确认在 `feat/byteui-standalone`，且只 `git add` 具体路径。若用 subagent 派发，dispatch 消息第一句必须是 `cd <worktree 绝对路径> && git branch --show-current`，Read/Edit 的 `file_path` 要带完整 worktree 绝对路径前缀。
- **byteui 不依赖 `dozer-core`/`dozer-app`/`dozer-client`，库代码里不得出现 Dozer 专属路径、环境变量名或业务约定**（`IconKind::Dozer` 品牌图标及其 SVG 是公开枚举成员，保留不动）。
- **dozer-app 的调用点一字不改**：`byteui::theme::font::body()` 等函数名与签名不变。
- **iced 版本固定 0.14**，dozer 与 byteui 必须一致，否则 `Element` 类型对不上。
- **消费方一律用 `tag` 引用 byteui，不用 `branch`。**
- **颜色/字号/几何/图标尺寸的默认值锁死**（`ColorTokens::byteboy2077()` 等），本计划不得改动任何数值。
- **所有 Cargo 命令用 `cargo`，不要加 `--offline` 之外的非常规 flag**；每次宣称通过前要贴出实际命令输出。
- **新建 GitHub 仓库、推送、打 tag 属于对外发布**：执行这些步骤前，先向用户确认仓库可见性（private/public）与组织归属，得到明确回答再执行。
- **前置：`dozer-silent1`（dozer 的一个 worktree，分支 `feat/silent-failures-batch1`，领先 main 8 个提交、落后 7 个，未推远程）不能被误删。** 本计划不会碰它；Task 4 里有一步验证它不改动 byteui，从而 rebase 到拆分后的 main 时不会冲突。

## Review Focus

1. **`BYTEUI_ICON_SCALE` / `DOZER_ICON_SCALE` 取值非法**（非数字、0、负数）：应被忽略并回落到 token 基准值，而不是 panic 或把缩放设成 0。两者同时存在时 `BYTEUI_ICON_SCALE` 优先；只有旧名时仍然生效（dozer 用户的现有设置不失效）。Task 1 的 `pick_env_scale` 测试覆盖。
2. **`init_scale` 在环境变量非法时**：旧代码只要变量存在就跳过落盘值读取；新行为是只有"有效覆盖值"才跳过。Task 1 测试覆盖，避免用户设了坏值后永远读不到自己保存的缩放。
3. **Badge 数字超上限**：`count_label(100, 99)` 必须是 `"99+"`，`count_label(99, 99)` 是 `"99"`，`count_label(0, 99)` 是 `"0"`。Task 5 测试覆盖。
4. **Tag 文字为空 / Button 禁用**：空标签不 panic；禁用的 Button 不产生消息（没有 `on_press`），样式变暗。Task 5 测试覆盖。
5. **导出的新仓库根目录结构**：`Cargo.toml` 必须在仓库根，历史里不应残留 dozer 其他目录；`v0.1.0` 必须能在一个全新克隆里 `cargo test` 通过。Task 3 的验证步骤覆盖。
6. **拆分后 dozer 的依赖图**：`cargo tree -d` 里 iced 系 crate 不应出现重复版本（重复意味着 `Element` 类型不兼容的隐患）。Task 4 验证。

---

## 文件结构总览

```
dozer 仓库（分支 feat/byteui-standalone）
├── crates/byteui/src/**                 # Task 1：注释、测试名、环境变量
├── crates/dozer-app/src/theme.rs        # Task 2：加一条锁值对照测试
├── crates/dozer-app/Cargo.toml          # Task 4：path → git 依赖
├── crates/byteui/                       # Task 4：删除
└── CLAUDE.md                            # Task 4：加一条说明

byteboyai/byteui 仓库（新）
├── Cargo.toml  LICENSE  README.md  CHANGELOG.md
├── .github/workflows/ci.yml             # Task 3
├── assets/icons/*.svg
├── src/{theme,interaction,layout,form,feedback,data}/
├── src/data/badge.rs  src/data/tag.rs   # Task 5
├── src/form/button.rs                   # Task 5
└── examples/gallery.rs                  # Task 6
```

---

### Task 1: 在 dozer 内把 byteui 清理到"无 dozer 专属内容"

**Files:**
- Modify: `crates/byteui/src/theme/icon_size.rs`（环境变量逻辑 + 测试 + 注释）
- Modify: `crates/byteui/src/theme/font.rs`、`geometry.rs`、`color.rs`（基线测试改名 + 注释）
- Modify: `crates/byteui/src/form/search_box.rs`、`feedback/toast.rs`、`interaction/icons.rs`、`interaction/context_menu.rs`（仅注释）

**Interfaces:**
- Produces: `icon_size` 内部新增私有函数 `pick_env_scale(primary: Option<String>, legacy: Option<String>) -> Option<f32>` 与 `env_scale_override() -> Option<f32>`；公开 API 签名不变。

- [ ] **Step 0: 建 worktree 与分支**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git worktree add .worktrees/byteui-standalone -b feat/byteui-standalone main
cd .worktrees/byteui-standalone && git branch --show-current
```
Expected: 输出 `feat/byteui-standalone`。后续所有 dozer 侧命令都在这个目录里执行。

- [ ] **Step 1: 写 `pick_env_scale` 的失败测试**

在 `crates/byteui/src/theme/icon_size.rs` 末尾的 `mod tests` 里追加：

```rust
    #[test]
    fn pick_env_scale_prefers_byteui_name() {
        assert_eq!(
            pick_env_scale(Some("1.5".into()), Some("2.0".into())),
            Some(1.5)
        );
    }

    #[test]
    fn pick_env_scale_falls_back_to_legacy_name() {
        assert_eq!(pick_env_scale(None, Some("2.0".into())), Some(2.0));
    }

    #[test]
    fn pick_env_scale_ignores_invalid_values() {
        // 非数字、0、负数都当作"没设"。
        assert_eq!(pick_env_scale(Some("abc".into()), None), None);
        assert_eq!(pick_env_scale(Some("0".into()), None), None);
        assert_eq!(pick_env_scale(Some("-1".into()), None), None);
        // 主名非法时仍可回落到旧名。
        assert_eq!(
            pick_env_scale(Some("abc".into()), Some("2.0".into())),
            Some(2.0)
        );
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p byteui pick_env_scale`
Expected: 编译失败，`cannot find function pick_env_scale`。

- [ ] **Step 3: 实现，并替换所有 `DOZER_ICON_SCALE` 的直接读取**

把 `base_scale()` 及其上方说明替换为：

```rust
/// 环境变量覆盖：`BYTEUI_ICON_SCALE` 优先；`DOZER_ICON_SCALE` 是 Dozer 早期
/// 使用的名字，仍然兼容（已有用户的设置不失效）。
const ENV_SCALE: &str = "BYTEUI_ICON_SCALE";
const ENV_SCALE_LEGACY: &str = "DOZER_ICON_SCALE";

/// 纯函数，便于不碰进程环境地测试：非数字、0、负数都当作"没设"。
fn pick_env_scale(primary: Option<String>, legacy: Option<String>) -> Option<f32> {
    let parse = |v: Option<String>| {
        v.and_then(|s| s.parse::<f32>().ok())
            .filter(|&v| v.is_finite() && v > 0.0)
    };
    parse(primary).or_else(|| parse(legacy))
}

fn env_scale_override() -> Option<f32> {
    pick_env_scale(
        std::env::var(ENV_SCALE).ok(),
        std::env::var(ENV_SCALE_LEGACY).ok(),
    )
}

/// 启动基准 scale：环境变量覆盖优先，否则用 token 的 `scale`。
fn base_scale() -> f32 {
    env_scale_override().unwrap_or(current().scale)
}
```

`init_scale` 里的

```rust
    if std::env::var("DOZER_ICON_SCALE").is_ok() {
        return;
    }
```
改为
```rust
    if env_scale_override().is_some() {
        return;
    }
```

测试 `init_applies_persisted_and_reset_clears_it` 开头的
```rust
        if std::env::var("DOZER_ICON_SCALE").is_ok() {
            return;
        }
```
改为
```rust
        if env_scale_override().is_some() {
            return;
        }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p byteui`
Expected: 全部 PASS（含新增的 3 个 `pick_env_scale_*`）。

- [ ] **Step 5: 基线测试改名**

```bash
cd crates/byteui/src/theme
sed -i '' 's/byteboy2077_matches_dozer_app_baseline/byteboy2077_locked_values/' font.rs icon_size.rs geometry.rs
grep -rn "dozer_app_baseline" . ; echo "exit=$?"
```
Expected: grep 无输出，`exit=1`。

- [ ] **Step 6: 重写注释，去掉对 dozer-app 路径的引用**

规则：库里的注释不得指向 `dozer-app` 的文件或函数；改成"调用方/应用侧"。重复出现的几处固定措辞：

| 原文 | 改为 |
|------|------|
| `` 供调用方（如 `dozer-app::theme::init()`）`` | `` 供调用方（如应用启动时的 `theme::init()`）`` |
| `` 逐一对应 `dozer-app` 当前 `assets/theme/workspace.json` 的…`` | `锁死为 ByteBoy2077 的取值：…`（保留后半句对 JSON 字段的描述，改成"与应用侧 JSON 同名字段一致"） |
| `` 防漂移锚：`byteboy2077()` 的每个字段值必须和 `dozer-app` 当前…一致`` | `` 锁值快照：`byteboy2077()` 的每个字段值不得随意改动；改动必须同步确认所有消费方。`` |

其余零散注释（`search_box.rs:2,5`、`toast.rs:3`、`color.rs:43,48,171`、`icon_size.rs:12,21-22,38,90,145,172`、`icons.rs:85,88,157-161,184,252`、`context_menu.rs:9-11,36`）逐条按同一规则改：把 `dozer-app` 换成"应用侧"，把 Dozer 专属路径/名词（`dozer-app::native_menu`、顶栏 "Dozer Home" 等）换成通用描述；**`IconKind::Dozer` 与 `assets/icons/dozer-logo.svg` 不改。**

- [ ] **Step 7: 验收 grep**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/byteui-standalone/crates/byteui
grep -rn "dozer-app\|dozer_app" src; echo "exit=$?"
grep -rniE "dozer" src
```
Expected: 第一条无输出（`exit=1`）；第二条只剩：`BYTEUI` 旁的 `ENV_SCALE_LEGACY`/`DOZER_ICON_SCALE` 兼容说明、`IconKind::Dozer`、`dozer-logo.svg`、描述该品牌图标的注释。

- [ ] **Step 8: 全量验证并提交**

Run: `cargo test -p byteui && cargo clippy -p byteui --all-targets && cargo fmt -p byteui -- --check`
Expected: 全部成功。

```bash
git branch --show-current   # 必须是 feat/byteui-standalone
git add crates/byteui
git commit -m "refactor(byteui): strip dozer-specific references ahead of repo split" \
  -m "Add BYTEUI_ICON_SCALE (DOZER_ICON_SCALE kept as legacy fallback), rename baseline tests to locked-value snapshots, reword comments." \
  -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: dozer-app 加一条锁值对照测试

**Files:**
- Modify: `crates/dozer-app/src/theme.rs`（`mod tests`）

**Interfaces:**
- Consumes: `byteui::theme::{font,geometry,icon_size}::{current, FontTokens::byteboy2077, …}`（Task 1 保持不变）

目的：拆库后 dozer-app 的 `workspace.json` 与 byteui 默认值的关系要有明确保证。目前二者相同；以后有意分叉时，必须同时改这条测试，从而留下痕迹。

- [ ] **Step 1: 写测试**

在 `crates/dozer-app/src/theme.rs` 的 `mod tests` 内追加：

```rust
    /// dozer-app 的 `workspace.json` 与 byteui 的 ByteBoy2077 默认值目前一致。
    /// 若将来有意让 Dozer 与默认值分叉，同时修改本测试，留下"有意分叉"的记录。
    #[test]
    fn workspace_json_matches_byteui_defaults() {
        let raw: RawWorkspaceFile =
            serde_json::from_str(WORKSPACE_JSON).expect("workspace.json 格式错误(解析失败)");
        assert_eq!(
            format!("{:?}", raw.font_sizes),
            format!("{:?}", byteui::theme::font::FontTokens::byteboy2077())
        );
        assert_eq!(
            format!("{:?}", raw.geometry),
            format!("{:?}", byteui::theme::geometry::GeometryTokens::byteboy2077())
        );
        assert_eq!(
            format!("{:?}", raw.icon_sizes),
            format!("{:?}", byteui::theme::icon_size::IconSizeTokens::byteboy2077())
        );
    }
```

- [ ] **Step 2: 运行**

Run: `cargo test -p dozer-app workspace_json_matches_byteui_defaults`
Expected: PASS。若 FAIL，说明 JSON 与默认值此前就有差异：**不要改数值**，把差异列给用户，并把测试改为只断言"当前已知差异字段"（需用户确认）。

若报 `GeometryTokens`/`IconSizeTokens` 没有 `Debug`：给对应结构体补 `#[derive(Debug)]`（byteui 内改动，属 Task 1 的延伸，同一分支提交）。

- [ ] **Step 3: 提交**

```bash
git branch --show-current
git add crates/dozer-app/src/theme.rs crates/byteui
git commit -m "test(theme): pin dozer workspace.json against byteui defaults" \
  -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: 导出历史、建 `byteboyai/byteui` 仓库、发 v0.1.0

**Files:**
- Create（新仓库）: `README.md`、`CHANGELOG.md`、`LICENSE`、`.github/workflows/ci.yml`
- Modify（新仓库）: `Cargo.toml`（补 `description`/`repository`）

**Interfaces:**
- Produces: 新仓库 tag `v0.1.0`（内容 = dozer 分支上 Task 1+2 之后的 `crates/byteui`），供 Task 4 引用。

- [ ] **Step 1: 安装 git-filter-repo（本机未装）**

Run: `brew install git-filter-repo && git filter-repo --version`
Expected: 打印版本号。

- [ ] **Step 2: 从分支导出带历史的克隆**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy
git clone --no-local --branch feat/byteui-standalone dozer byteui
cd byteui
git filter-repo --subdirectory-filter crates/byteui
ls; git log --oneline | wc -l; git log --oneline -3
```
Expected: `ls` 看到 `Cargo.toml assets src`（在仓库根）；提交数约 75（原 73 + Task 1、2 各一）；`git log` 最近的是 Task 1/2 的提交。`filter-repo` 会移除 `origin` 远程，属正常。

- [ ] **Step 3: 补仓库文件**

`Cargo.toml` 的 `[package]` 增加：

```toml
description = "ByteBoy 系产品共用的 iced 0.14 UI 组件库"
repository = "https://github.com/byteboyai/byteui"
```

`LICENSE`：MIT 全文，版权行 `Copyright (c) 2026 ByteBoy`。

`CHANGELOG.md`：

```markdown
# Changelog

## 0.1.0

- 自 dozer 仓库的 `crates/byteui` 拆出，带完整提交历史；内容与拆出前一致。
- 新增环境变量 `BYTEUI_ICON_SCALE`；`DOZER_ICON_SCALE` 作为旧名继续兼容。
```

`README.md`（要点全部写进去）：

```markdown
# byteui

ByteBoy 系产品（dozer、digger）共用的 iced 0.14 组件库。组件命名对齐 amis 的分类与名字。

## 使用

    byteui = { git = "https://github.com/byteboyai/byteui", tag = "v0.1.0" }

一律用 `tag`，不要用 `branch`。

## iced 版本

所有消费方必须与本库使用同一个 iced 版本（当前 0.14），否则 `Element` 类型不兼容。
升级 iced 时 byteui、dozer、digger 一起升。

## 主题

默认值是 ByteBoy2077。应用在启动时调用 `theme::{color,font,geometry,icon_size}::set_theme()`
整体替换。`icon_size::{init,persist,reset}_scale` 的落盘路径由应用传入。
环境变量 `BYTEUI_ICON_SCALE` 可覆盖启动缩放（`DOZER_ICON_SCALE` 为旧名，仍兼容）。

## 本地联调（不提交）

在消费方的 `.cargo/config.toml` 里加：

    [patch."https://github.com/byteboyai/byteui"]
    byteui = { path = "../byteui" }

## 什么组件可以进库

1. 至少两个项目需要，或设计稿里已明确出现；只有一个项目用的留在该应用里。
2. 命名沿用 amis 组件名。
3. 组件无状态：`fn(参数) -> Element`；颜色、字号、间距一律读 token，不写死数值。
4. 每个组件有最小测试，覆盖各种状态。

## 版本

语义化版本。加组件是 minor；改已有签名是 breaking（0.x 阶段也要在 CHANGELOG 写明）。
```

`.github/workflows/ci.yml`：

```yaml
name: ci
on:
  push:
    branches: [main]
  pull_request:
jobs:
  test:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
```

- [ ] **Step 4: 在全新目录验证能独立构建**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/byteui
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
```
Expected: 全部成功。若 `-D warnings` 暴露 dozer 里没被当作错误的警告，在本仓库内逐个修（不改行为）。

- [ ] **Step 5: 提交，然后向用户确认再发布**

```bash
git add -A && git commit -m "chore: add README, CHANGELOG, LICENSE and CI for standalone repo" \
  -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git tag v0.1.0
```

**停下来问用户**：仓库 `byteboyai/byteui` 设为 private 还是 public？当前 `gh` 登录账号是 `chrischiangs`，是否有权限在 `byteboyai` 组织下建库？得到明确回答后：

```bash
gh repo create byteboyai/byteui --private --source . --remote origin --push   # 或 --public
git push origin v0.1.0
gh run watch   # 等 CI 通过
```
Expected: 仓库创建成功，CI 绿。

- [ ] **Step 6: 用全新克隆验证 tag 可用**

```bash
rm -rf /tmp/byteui-verify && git clone --branch v0.1.0 https://github.com/byteboyai/byteui /tmp/byteui-verify
cd /tmp/byteui-verify && cargo test
```
Expected: PASS。（private 仓库需要本机有 GitHub 凭据，这一步同时验证了 cargo 拉取 git 依赖所需的凭据可用；若 cargo 拉取失败，在 `~/.cargo/config.toml` 加 `[net] git-fetch-with-cli = true`。）

---

### Task 4: dozer 切到 git 依赖，删除 `crates/byteui`

**Files:**
- Modify: `crates/dozer-app/Cargo.toml:23`
- Delete: `crates/byteui/`
- Modify: `CLAUDE.md`
- Modify: `Cargo.lock`（自动）

**Interfaces:**
- Consumes: Task 3 的 `byteui` tag `v0.1.0`。

- [ ] **Step 1: 确认 silent1 分支不碰 byteui（rebase 不会冲突）**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer
git diff --stat main...feat/silent-failures-batch1 | grep -i byteui; echo "exit=$?"
```
Expected: 无输出，`exit=1`。若有输出，停下来报告，说明该分支改了 byteui，需先合并它。

- [ ] **Step 2: 改依赖并删除本地 crate**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/dozer/.worktrees/byteui-standalone
git branch --show-current
```
把 `crates/dozer-app/Cargo.toml` 的

```toml
byteui = { path = "../byteui" }
```
改为

```toml
byteui = { git = "https://github.com/byteboyai/byteui", tag = "v0.1.0" }
```

```bash
git rm -r crates/byteui
cargo update -p byteui 2>&1 | tail -3
```

- [ ] **Step 3: 全 workspace 验证**

```bash
cargo build 2>&1 | tail -3
cargo test -p dozer-app 2>&1 | tail -5
cargo clippy --all-targets 2>&1 | tail -3
cargo tree -d 2>&1 | grep -E "^iced|^iced_" ; echo "dup-iced exit=$?"
```
Expected: build、test、clippy 成功；最后一条无输出（`exit=1`），即 iced 系 crate 没有重复版本。若 `cargo test -p dozer-app` 因本分支之外的原因失败，对比 main 上同命令结果，只报告新增失败。

- [ ] **Step 4: 在 CLAUDE.md 记录迁移**

在 `## 关键裁决（违反即错）` 列表中，"新增/改造 icon 按钮、tab 类 UI 时优先复用统一组件" 一条之后加：

```markdown
- **byteui 已迁到独立仓库 `byteboyai/byteui`（2026-10-01 起）**，dozer 与 digger 共用，通过 `byteui = { git = "...", tag = "vX.Y.Z" }` 引用，一律用 tag。改组件要去 byteui 仓库改并发版；本地联调在 `.cargo/config.toml` 里 `[patch."https://github.com/byteboyai/byteui"] byteui = { path = "../byteui" }`，不提交。历史 plan/spec 里的 `crates/byteui/...` 路径指的是拆分前的位置。iced 版本必须与 byteui 一致，升级时一起升。设计见 `docs/superpowers/specs/2026-10-01-byteui-standalone-repo-design.md`。
```

- [ ] **Step 5: 提交**

```bash
git branch --show-current
git add Cargo.lock crates/dozer-app/Cargo.toml CLAUDE.md
git commit -m "refactor: consume byteui as a git dependency (byteboyai/byteui v0.1.0)" \
  -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
注意：删除 `crates/byteui` 已由 `git rm` 暂存，会随此提交一起进入；提交前 `git status --short` 确认只有上述路径与 `crates/byteui` 的删除。

- [ ] **Step 6: 请用户审阅分支，合并由用户决定**

不要自己合并到 main。报告：分支名、验证输出、以及"`dozer-silent1` 的分支在本分支合并后需要 `git rebase main`（Step 1 已验证不冲突）"。

---

### Task 5: 在 byteui 仓库加 Badge、Tag、Button，发 v0.2.0

**Files（均在 `/Users/chrischiang/Projects/CoralProjects/byteboy/byteui`）:**
- Create: `src/data/badge.rs`、`src/data/tag.rs`、`src/form/button.rs`
- Modify: `src/data/mod.rs`、`src/form/mod.rs`、`CHANGELOG.md`、`Cargo.toml`（版本）

**Interfaces:**
- Produces:
  - `data::tag::Tone { Neutral, Gold, Cyan, Green, Red }`（`Clone, Copy, PartialEq, Debug`）
  - `data::tag::tone_color(tone: Tone, colors: &ColorTokens) -> Color`
  - `data::tag::view<'a, Message: 'a>(text: &'a str, tone: Tone) -> Element<'a, Message, Theme, Renderer>`
  - `data::badge::count_label(count: u32, max: u32) -> String`
  - `data::badge::count<'a, Message: 'a>(count: u32, max: u32) -> Element<…>`（数字角标）
  - `data::badge::dot<'a, Message: 'a>() -> Element<…>`（红点角标）
  - `form::button::Kind { Primary, Secondary }`（`Clone, Copy, PartialEq, Debug`）
  - `form::button::view<'a, Message: Clone + 'a>(label: &'a str, kind: Kind, leading: Option<Element<'a, Message, Theme, Renderer>>, on_press: Option<Message>) -> Element<…>`

全程在分支 `feat/badge-tag-button` 上做：`git switch -c feat/badge-tag-button`。

- [ ] **Step 1: 写 Tag 的失败测试**

创建 `src/data/tag.rs`：

```rust
//! amis `tag`(标签):<https://baidu.github.io/amis/zh-CN/components/tag>
//! 带文字的小标签,如"可审查""冲突""仅本地"。颜色用 `Tone` 选语义色,
//! 底色是语义色与面板色按固定比例混合,不写死色值。

use iced_widget::core::{Border, Color, Element};
use iced_widget::{container, text};

use crate::theme::color::{ColorTokens, current, mix};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone {
    Neutral,
    Gold,
    Cyan,
    Green,
    Red,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn tone_color_maps_to_semantic_tokens() {
        let c = ColorTokens::byteboy2077();
        assert_eq!(tone_color(Tone::Gold, &c), c.gold);
        assert_eq!(tone_color(Tone::Cyan, &c), c.cyan);
        assert_eq!(tone_color(Tone::Green, &c), c.green);
        assert_eq!(tone_color(Tone::Red, &c), c.red);
        assert_eq!(tone_color(Tone::Neutral, &c), c.dim);
    }

    #[test]
    fn empty_text_constructs_without_panic() {
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = view("", Tone::Neutral);
    }

    #[test]
    fn every_tone_constructs_without_panic() {
        for tone in [Tone::Neutral, Tone::Gold, Tone::Cyan, Tone::Green, Tone::Red] {
            let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = view("可审查", tone);
        }
    }
}
```

在 `src/data/mod.rs` 加 `pub mod tag;`（按字母序放在 `property` 之后）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test data::tag`
Expected: 编译失败，`cannot find function tone_color` / `view`。

- [ ] **Step 3: 实现 Tag**

在 `tag.rs` 的 `Tone` 定义之后、`#[cfg(test)]` 之前加：

```rust
pub fn tone_color(tone: Tone, colors: &ColorTokens) -> Color {
    match tone {
        Tone::Neutral => colors.dim,
        Tone::Gold => colors.gold,
        Tone::Cyan => colors.cyan,
        Tone::Green => colors.green,
        Tone::Red => colors.red,
    }
}

pub fn view<'a, Message: 'a>(
    text_content: &'a str,
    tone: Tone,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = current();
    let accent = tone_color(tone, &colors);
    let bg = mix(colors.panel, accent, 0.18);
    let border = mix(colors.panel, accent, 0.45);
    container(
        text(text_content)
            .size(crate::theme::font::caption_sm())
            .color(accent),
    )
    .padding([2.0, 8.0])
    .style(move |_theme: &iced_widget::Theme| container::Style {
        background: Some(bg.into()),
        border: Border {
            color: border,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}
```

- [ ] **Step 4: 跑测试通过并提交**

Run: `cargo test data::tag`
Expected: 3 个 PASS。

```bash
git add src/data/tag.rs src/data/mod.rs
git commit -m "feat(data): add Tag component" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: 写 Badge 的失败测试**

创建 `src/data/badge.rs`：

```rust
//! amis `badge`(角标):<https://baidu.github.io/amis/zh-CN/components/badge>
//! 数字角标(如"待审变更 3")与红点角标。超过上限显示 `99+` 这类写法。
//! 是否显示(例如数量为 0 时隐藏)由调用方决定,组件只负责渲染。

use iced_widget::core::{Border, Element};
use iced_widget::{container, text};

use crate::theme::color::current;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn count_label_caps_at_max() {
        assert_eq!(count_label(0, 99), "0");
        assert_eq!(count_label(99, 99), "99");
        assert_eq!(count_label(100, 99), "99+");
        assert_eq!(count_label(5, 9), "5");
        assert_eq!(count_label(10, 9), "9+");
    }

    #[test]
    fn badge_components_construct_without_panic() {
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = count(3, 99);
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = count(0, 99);
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = dot();
    }
}
```

在 `src/data/mod.rs` 加 `pub mod badge;`（放在 `card` 之前，保持字母序）。

- [ ] **Step 6: 跑测试确认失败**

Run: `cargo test data::badge`
Expected: 编译失败，`cannot find function count_label`。

- [ ] **Step 7: 实现 Badge**

```rust
/// 数字显示文本:超过 `max` 显示 `{max}+`。
pub fn count_label(count: u32, max: u32) -> String {
    if count > max {
        format!("{max}+")
    } else {
        count.to_string()
    }
}

/// 数字角标:金色底、深色字,用于"待审变更 3"这类需要引起甲方注意的计数。
pub fn count<'a, Message: 'a>(
    count: u32,
    max: u32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = current();
    container(
        text(count_label(count, max))
            .size(crate::theme::font::caption_sm())
            .color(colors.panel),
    )
    .padding([1.0, 6.0])
    .style(move |_theme: &iced_widget::Theme| container::Style {
        background: Some(colors.gold.into()),
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// 红点角标:只表示"有新内容",不带数字。
pub fn dot<'a, Message: 'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = current();
    let size = crate::theme::font::dot_sm() as f32 * 0.8;
    container(iced_widget::Space::new())
        .width(size)
        .height(size)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(colors.red.into()),
            border: Border {
                radius: (size / 2.0).into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}
```

`iced_widget::Space::new()` 在 0.14 的签名若不同（有的版本是 `Space::new(width, height)`），以 `cargo build` 报错为准，用 `Space::with_width/height` 或固定宽高的 `container` 代替，**行为保持不变：一个 `size`×`size` 的圆点**。

- [ ] **Step 8: 跑测试通过并提交**

Run: `cargo test data::badge && cargo clippy --all-targets -- -D warnings`
Expected: PASS，无警告。

```bash
git add src/data/badge.rs src/data/mod.rs
git commit -m "feat(data): add Badge component" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 9: 写 Button 的失败测试**

创建 `src/form/button.rs`：

```rust
//! amis `button`(按钮):<https://baidu.github.io/amis/zh-CN/components/button>
//! 金色主按钮(甲方的关键动作,如"批准并提交""打开项目")与次要描边按钮
//! (如"拒绝")。`on_press` 为 `None` 即禁用:不产生消息,样式变暗。

use iced_widget::button::{self, Status};
use iced_widget::core::{Border, Color, Element, Padding};
use iced_widget::{Row, text};

use crate::theme::color::{ColorTokens, current, mix};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Primary,
    Secondary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {
        Pressed,
    }

    #[test]
    fn primary_uses_gold_background_and_dark_text() {
        let c = ColorTokens::byteboy2077();
        let s = style_for(Kind::Primary, Status::Active, &c);
        assert_eq!(s.background, Some(c.gold.into()));
        assert_eq!(s.text_color, c.panel);
    }

    #[test]
    fn secondary_is_outlined_with_transparent_background() {
        let c = ColorTokens::byteboy2077();
        let s = style_for(Kind::Secondary, Status::Active, &c);
        assert_eq!(s.background, None);
        assert_eq!(s.border.color, c.border);
        assert_eq!(s.text_color, c.cream);
    }

    #[test]
    fn disabled_is_dimmed_for_both_kinds() {
        let c = ColorTokens::byteboy2077();
        for kind in [Kind::Primary, Kind::Secondary] {
            let s = style_for(kind, Status::Disabled, &c);
            assert_eq!(s.text_color, c.dim, "{kind:?}");
        }
    }

    #[test]
    fn constructs_enabled_disabled_and_with_leading() {
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            view("批准并提交", Kind::Primary, None, Some(Msg::Pressed));
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            view("拒绝", Kind::Secondary, None, None);
        let icon: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> =
            iced_widget::text("+").into();
        let _ = view("新建项目", Kind::Primary, Some(icon), Some(Msg::Pressed));
    }
}
```

在 `src/form/mod.rs` 加 `pub mod button;`（放在最前，保持字母序）。

- [ ] **Step 10: 跑测试确认失败**

Run: `cargo test form::button`
Expected: 编译失败，`cannot find function style_for` / `view`。

- [ ] **Step 11: 实现 Button**

```rust
/// 纯函数:状态 → 样式,便于不起窗口地测试。
pub fn style_for(kind: Kind, status: Status, colors: &ColorTokens) -> button::Style {
    let radius = 6.0;
    match (kind, status) {
        (Kind::Primary, Status::Disabled) => button::Style {
            background: Some(mix(colors.panel, colors.gold, 0.25).into()),
            text_color: colors.dim,
            border: Border { radius: radius.into(), ..Border::default() },
            ..button::Style::default()
        },
        (Kind::Primary, status) => {
            let bg = match status {
                Status::Hovered | Status::Pressed => mix(colors.gold, Color::WHITE, 0.15),
                _ => colors.gold,
            };
            button::Style {
                background: Some(bg.into()),
                text_color: colors.panel,
                border: Border { radius: radius.into(), ..Border::default() },
                ..button::Style::default()
            }
        }
        (Kind::Secondary, Status::Disabled) => button::Style {
            background: None,
            text_color: colors.dim,
            border: Border { color: colors.border, width: 1.0, radius: radius.into() },
            ..button::Style::default()
        },
        (Kind::Secondary, status) => {
            let border = match status {
                Status::Hovered | Status::Pressed => colors.gold,
                _ => colors.border,
            };
            button::Style {
                background: None,
                text_color: colors.cream,
                border: Border { color: border, width: 1.0, radius: radius.into() },
                ..button::Style::default()
            }
        }
    }
}

pub fn view<'a, Message: Clone + 'a>(
    label: &'a str,
    kind: Kind,
    leading: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
    on_press: Option<Message>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut content = Row::new().spacing(6).align_y(iced_widget::core::alignment::Vertical::Center);
    if let Some(icon) = leading {
        content = content.push(icon);
    }
    content = content.push(text(label).size(crate::theme::font::label()));
    iced_widget::button(content)
        .padding(Padding::from([6.0, 14.0]))
        .on_press_maybe(on_press)
        .style(move |_theme: &iced_widget::Theme, status: Status| {
            style_for(kind, status, &current())
        })
        .into()
}
```

`Row::align_y` 的签名与 iced 0.14 是否一致以 `cargo build` 为准（早期版本叫 `align_items`）；若不同，换成 0.14 的等价写法，行为保持"内容垂直居中"。

- [ ] **Step 12: 跑测试、clippy、fmt，提交**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: 全部成功。

```bash
git add src/form/button.rs src/form/mod.rs
git commit -m "feat(form): add Button component (primary/secondary/disabled)" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 13: 版本、CHANGELOG，发 v0.2.0**

`Cargo.toml` 的 `version` 改为 `0.2.0`。`CHANGELOG.md` 顶部加：

```markdown
## 0.2.0

- 新增 `data::tag`（带文字标签，`Tone` 选语义色）。
- 新增 `data::badge`（数字角标与红点角标）。
- 新增 `form::button`（Primary / Secondary / 禁用态，可带前置图标）。
- 这是对 2026-08-18 byteui 计划中"不做 Badge/Tag"的修订：digger 设计稿已明确需要。
```

```bash
git add Cargo.toml CHANGELOG.md
git commit -m "chore: release 0.2.0" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin feat/badge-tag-button
```
CI 通过后**请用户确认合并**，合并到 `main` 后：

```bash
git switch main && git pull && git tag v0.2.0 && git push origin v0.2.0
```

---

### Task 6: 组件展示示例 `examples/gallery.rs`

**Files（`byteui` 仓库）:**
- Create: `examples/gallery.rs`
- Modify: `Cargo.toml`（dev-dependencies）、`README.md`

**Interfaces:**
- Consumes: Task 5 的 `Tag`、`Badge`、`Button`，以及已有的 `form::switch::view`、`data::card::view`、`feedback::status::dot`。

分支 `feat/gallery`。

- [ ] **Step 1: 加 dev 依赖**

`Cargo.toml` 的 `[dev-dependencies]` 增加：

```toml
iced = { version = "0.14", features = ["svg", "canvas"] }
```

- [ ] **Step 2: 写示例**

```rust
//! 组件展示:把各组件的各种状态摆在一个窗口里,改组件后在这里目测。
//! 运行:`cargo run --example gallery`

use byteui::data::{badge, card, tag};
use byteui::feedback::status;
use byteui::form::{button, switch};
use byteui::theme::color;
use iced::widget::{column, container, row, text};
use iced::{Element, Length};

#[derive(Default)]
struct Gallery {
    switch_on: bool,
}

#[derive(Clone)]
enum Msg {
    Toggled(bool),
    Noop,
}

impl Gallery {
    fn update(&mut self, msg: Msg) {
        if let Msg::Toggled(v) = msg {
            self.switch_on = v;
        }
    }

    fn view(&self) -> Element<'_, Msg> {
        let c = color::current();
        let title = |s: &'static str| text(s).size(14).color(c.cream);
        let body = column![
            title("Button"),
            row![
                button::view("批准并提交", button::Kind::Primary, None, Some(Msg::Noop)),
                button::view("拒绝", button::Kind::Secondary, None, Some(Msg::Noop)),
                button::view("已禁用", button::Kind::Primary, None, None),
                button::view("已禁用", button::Kind::Secondary, None, None),
            ]
            .spacing(8),
            title("Tag"),
            row![
                tag::view("可审查", tag::Tone::Green),
                tag::view("冲突", tag::Tone::Red),
                tag::view("仅本地", tag::Tone::Cyan),
                tag::view("r128", tag::Tone::Gold),
                tag::view("v7", tag::Tone::Neutral),
            ]
            .spacing(8),
            title("Badge"),
            row![badge::count(3, 99), badge::count(120, 99), badge::dot()].spacing(8),
            title("Status / Switch / Card"),
            row![
                status::dot(c.green),
                status::dot(c.gold),
                status::dot(c.red),
                switch::view("开关", self.switch_on, Msg::Toggled),
            ]
            .spacing(8),
            card::view("Project Memory", Some("r128 · 健康"), false, false),
        ]
        .spacing(14);
        container(body)
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(c.panel.into()),
                ..container::Style::default()
            })
            .into()
    }
}

fn main() -> iced::Result {
    iced::application(Gallery::default, Gallery::update, Gallery::view)
        .theme(|_| iced::Theme::Dark)
        .run()
}
```

若 `iced::application` 在 0.14 的参数形式与上面不同，以 `cargo build --example gallery` 的报错为准调整入口，不改展示内容。

- [ ] **Step 3: 构建并目测**

Run: `cargo build --example gallery && cargo run --example gallery`
Expected: 构建成功；窗口里能看到 Button 四种状态、Tag 五种语气、Badge 三种、状态点与开关、一张卡片，深色底。**把窗口截图或描述贴给用户确认**，不要只凭"构建通过"就宣称完成。

- [ ] **Step 4: README 加一行并提交**

README 的"本地联调"之前加：

```markdown
## 组件展示

`cargo run --example gallery` 查看全部组件的各种状态；改组件后用它目测。
```

```bash
cargo clippy --all-targets -- -D warnings && cargo fmt --check
git add examples/gallery.rs Cargo.toml README.md
git commit -m "docs: add component gallery example" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin feat/gallery
```
请用户确认后合并；gallery 属于示例，不单独发版（随下一个版本一起发布）。

---

## 自检记录

- **Spec 覆盖**：§2 仓库形状 → Task 3；§3 进库/留应用 + 两处改动 → Task 1、2（另加环境变量名一处，spec 已补记）；§4 新组件与准入规则 → Task 5、README；§5 迁移步骤 1–6 → Task 1–5；§6 CI 与 gallery → Task 3、6；§7 风险（并发、worktree、iced 一致、tag）→ Global Constraints 与 Task 4 验证步骤。`dozer-silent1` 的处置 → Global Constraints 与 Task 4 Step 1。
- **类型一致**：`Tone`/`tone_color`/`count_label`/`Kind`/`style_for` 在测试与实现里同名同签名；Task 6 只使用 Task 5 的公开函数。
- **已知需要以编译器为准的点**（已在对应步骤标明，行为保持不变）：iced 0.14 中 `Space::new`、`Row::align_y`、`iced::application` 的确切签名。
