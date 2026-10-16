# V2 收口三件事:Agent 单点表 / 三仓依赖对齐 / 失败基线清零 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不动产品内核的前提下,把 V2 底座线收口:①清掉已知的红色基线,让门禁可信;②把 AgentKind 的"手写清单"收成一张编译器盯得住的表;③让 digger 能脱离本地 `[patch]`、与 dozer 用同一套已发布依赖构建。

**Architecture:** 三部分互相独立,执行顺序 **Part 3 → Part 1 → Part 2**(先让红灯变绿,后续改动才有可信的回归信号;Part 2 涉及推送远端,需要用户确认,放最后)。Part 1 只做 `dozer-model`/`dozer-core`/`dozerd`/`dozer-app` 内的纯重构,行为不变,唯一例外是顺手修一个已被证实的 bug(项目删除时漏删 v8agent 会话)。Part 2 跨 `dozer` 与 `digger` 两个仓库。

**Tech Stack:** Rust workspace(iced 0.14、rusqlite)、Python 审计脚本(`scripts/audit`)、cargo git 依赖(bytehost/byteui/bytegit 按 tag)。

**Spec:** 无单独 spec;依据以下三份现状文档(执行者需先读):
- `docs/dozer-v2/agent-pillar-audit.md`(Q15,Agent 分布审计与"第一刀"范围)
- `docs/dozer-v2/digger-bytehost-integration/baseline.md` §4 与 `acceptance.md` §4(既有失败基线)
- `docs/dozer-v2/bytehost-H8-evaluation.md`(H8 暂缓的结论,本计划不碰)

## Global Constraints

- 分支:dozer 在**独立 worktree 分支** `feat/v2-hygiene` 上做(`git worktree add ../dozer-v2-hygiene -b feat/v2-hygiene`),审阅后再合并 main;`main` 上常有并发会话的提交与 WIP,**禁止直接在 main 工作树里改**。Digger 改动在 digger 仓库的分支 `chore/align-deps-2026-10`(digger 无远端)。
- 所有 dispatch 给子 agent 的命令必须带 worktree 的绝对路径前缀(历史上子 agent 会漂回 main)。
- worktree 里**没有** `.cargo/config.toml`(它被 gitignore,只在主 checkout 存在),所以 worktree 里的构建走的是已发布 tag——这正是 Part 2 要验证的"干净解析"状态,不要把主 checkout 的本地 patch 拷进来。
- 日志:`dozerd`/`dozer-app` 禁止裸 `tracing::*!`/`eprintln!`,用 `dozer_core::log_*!`(`scripts/check-log-scope.sh` 门禁)。
- `dozer-model` 只许依赖 `serde`(见其 `Cargo.toml` 注释),Part 1 加的代码不得引入其他依赖。
- `label()` 的返回值是**持久化名**(写进 SQLite),任何改动都会让已有数据读不回;本计划**不得改动任何现有 label 字符串**(Task 4 有测试钉死)。
- 面板边界门禁基线只降不升:`python3 scripts/audit/check_panel_boundary.py` 必须保持 `ok`。
- 字体、Toast、Figma 等 UI 规则本计划不涉及。
- commit 信息末尾加:`Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- commit 前必须 `git status` + `git diff --cached --stat` 看 staged 全貌(并发会话会把无关改动卷进 index)。
- 外部可见动作(`git push`、推 tag)**必须先向用户确认**,见 Task 9。

## Review Focus

1. **持久化名漂移**:任何 `AgentKind` 的字符串映射改动,必须保证旧库里已写入的 `"claude"/"codebuddy"/…/"qodercn"` 以及 transcripts 里的 `"unknown"` 仍读得回(Task 4/5 有往返与钉死测试)。
2. **v8agent 漏删**:项目删除路径用手写五家清单,漏了 v8agent;改成表驱动后行为会"多删"v8agent 会话——这是预期修复,但要有测试证明删的是**该项目**的、不波及别的项目(Task 6)。
3. **枚举顺序依赖**:`scan.rs` 的现有测试断言 `found[0]` 是 Claude、`found[1]` 是 Codebuddy;`AgentKind::ALL` 的顺序一变这些测试就红(Task 6 保持原相对顺序)。
4. **同源 git 依赖出现两份**:dozer 内部 `byteui` 在 `dozer-panel-todo` 是 v0.4.3、其余是 v0.4.4;本地 `[patch]` 把它们压成一份所以没暴露,digger(无 patch)里会变成两份 `byteui`,主题全局状态分裂(Task 10 修并加门禁)。
5. **推送后不可撤回**:push 的是 main 上全部 25 个未推送提交(不止 digger 要用的);确认前不得推送(Task 9)。

---

# Part 3:失败基线清零(先做)

现状实测(2026-10-16,本 plan 起草时):
- `cargo test -p dozer-app --bin dozer delete_confirm_spec_reflects_pending_target` → **已通过**(基线文档里的"历史失败"已过期,只需改文档)。
- `usage_probe::host::tests::host_site_citations_point_at_the_code_they_name` → **失败**:`BuildActivationCtx: app/update.rs:5484 实际是 ssh::reload_from_disk(...)`。原因是该测试按**行号**核对 `host_site` 引用,`update.rs` 一变就漂移。
- `scripts/audit` 的 `test_unit_edges.ScanUsage` 两项 → **失败**:Usage 面板已搬到 `dozer-panel-usage`,`ext:usage` 现在只剩 `host_adapters/usage_panel` 一条边。
- `cargo fmt --all --check` → 59 处 diff;`cargo clippy --workspace --all-targets` → 约 14 类 warning。

### Task 1: usage_probe 引用核对改为"文件内含该代码"

**Files:**
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(测试 `host_site_citations_point_at_the_code_they_name`,约 270–320 行)

**Interfaces:**
- Consumes: `Duty::host_site() -> &'static str`(形如 `"app/update.rs:5484 + app/app.rs:123"`,不改)。
- Produces: 无新接口。

- [ ] **Step 1: 确认当前失败**

Run: `cd <worktree> && cargo test -p dozer-app --bin dozer host_site_citations 2>&1 | grep -A3 panicked`
Expected: FAIL,信息含 `BuildActivationCtx: app/update.rs:5484`

- [ ] **Step 2: 改测试体**

把测试里 `for (site, needle) in sites.iter().zip(needles.iter()) { ... }` 内部"按行号取行再 `contains`"改成"整个文件 `contains`"。具体:删掉 `rest`/`line` 的解析与 `text` 取行,保留 `file` 与 `src` 的 `match`,断言改为:

```rust
for (site, needle) in sites.iter().zip(needles.iter()) {
    let (file, _line) = site.split_once(':').expect("host_site 形如 file:line");
    let src = match file {
        "app/update.rs" => update,
        "app/app.rs" => app,
        "runtime.rs" => runtime,
        "extensions/usage.rs" | "host_adapters/usage_panel.rs" => usage,
        other => panic!("{duty:?}: 未知文件 {other}"),
    };
    // 行号会随无关编辑漂移(update.rs 6k+ 行),所以只核对"该文件里仍有这段代码";
    // 行号只是给人看的路标,不再是断言的一部分。
    assert!(
        src.contains(needle),
        "{duty:?}: {site} 所指文件里已找不到 `{needle}`,夹具需与真实 host 代码同步"
    );
}
```
并把该测试上方的文档注释里"按行号"的措辞改为"按文件内容"。

- [ ] **Step 3: 跑通**

Run: `cargo test -p dozer-app --bin dozer usage_probe 2>&1 | grep -E "test result|FAILED"`
Expected: `test result: ok. 27 passed`

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/usage_probe/host.rs
git commit -m "test(usage-probe): check host_site citations by file content, not line number"
```

### Task 2: 审计脚本跟上 Usage 面板搬家

**Files:**
- Modify: `scripts/audit/unit_edges.py`(`INTERNAL` 表)
- Modify: `scripts/audit/test_unit_edges.py`(`ScanUsage` 两个测试)

**Interfaces:** 无新接口。

- [ ] **Step 1: 确认失败**

Run: `python3 -m unittest discover -s scripts/audit -p 'test_*.py' 2>&1 | tail -4`
Expected: `FAILED (failures=2)`

- [ ] **Step 2: 分类新边**

在 `unit_edges.py` 的 `INTERNAL` 字典加一行(它是 host 给面板的适配器,属已有契约 A 类):

```python
    ("host_adapters", "usage_panel"): "A",
```

- [ ] **Step 3: 改断言**

`test_unit_edges.py` 中 `test_usage_depends_on_workspace_helpers` 改为:

```python
    def test_usage_depends_only_on_its_host_adapter(self):
        # Usage 面板已搬到 dozer-panel-usage;dozer-app 内只剩 host 适配层一条边。
        mods = {(r[1], r[2]) for r in unit_edges.scan("ext:usage")}
        self.assertIn(("host_adapters", "usage_panel"), mods)
```

- [ ] **Step 4: 跑通**

Run: `python3 -m unittest discover -s scripts/audit -p 'test_*.py' 2>&1 | tail -3`
Expected: `Ran 45 tests ... OK`;再跑 `python3 scripts/audit/check_panel_boundary.py` → `ok`。

- [ ] **Step 5: Commit**

```bash
git add scripts/audit/unit_edges.py scripts/audit/test_unit_edges.py
git commit -m "test(audit): ext:usage edges now go through host_adapters::usage_panel"
```

### Task 3: rustfmt 收口 + 基线文档更新

**Files:**
- Modify: 由 `cargo fmt` 改动的文件(59 处,多在 `dozer-panel-host`、`dozer-panel-todo`、`dozer-app/host_adapters`)
- Modify: `docs/dozer-v2/digger-bytehost-integration/baseline.md`(§4)、`docs/dozer-v2/digger-bytehost-integration/acceptance.md`(§1 末尾"既有失败基线"一段)

- [ ] **Step 1: 确认 worktree 干净再格式化**

Run: `git status --short`(应为空)→ `cargo fmt --all` → `git diff --stat | tail -1`
Expected: 仅有格式变化,无语义改动(抽查 `git diff -w --stat` 应远小于 `git diff --stat`,或为空)。

- [ ] **Step 2: 验证**

Run: `cargo fmt --all --check && echo FMT_OK` → `FMT_OK`;`cargo build --workspace 2>&1 | tail -2` → 通过。

- [ ] **Step 3: 单独 commit(纯格式,便于 blame 跳过)**

```bash
git add -u
git commit -m "style: cargo fmt --all"
```

- [ ] **Step 4: 更新基线文档**

在 `acceptance.md` §1 末尾"既有失败基线"段落后追加一行,并同步 `baseline.md` §4:

```markdown
> **2026-10-16 更新:** 上述三项历史失败已清零(`delete_confirm_spec…` 已自行转绿;`host_site_citations…` 改为按文件内容核对;`ScanUsage` 跟随 Usage 面板搬到 `dozer-panel-usage`)。`cargo fmt --all --check` 现为 clean。clippy 仍有约 14 类 warning(死代码与风格类,未纳入本次),基线数字见 `cargo clippy --workspace --all-targets 2>&1 | grep -c '^warning'`。
```
先跑一次该命令把真实数字写进去。

- [ ] **Step 5: 全量回归并 commit**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|FAILED|failed"` ;`python3 -m unittest discover -s scripts/audit -p 'test_*.py'` ;`bash scripts/check-log-scope.sh`
Expected: 全部通过。
```bash
git add docs/dozer-v2/digger-bytehost-integration/
git commit -m "docs(v2): clear historical failure baseline"
```

---

# Part 1:Agent 单点表(第一刀,范围见 Q15 §5)

**范围裁决(与审计文档一致,再次收紧):** 只做"身份/持久化名 + 路径目录"两类**静态事实**的收口。不碰 transcript 解析器、hook 安装、headless 调用、MCP 挂载、PTY。本次**不**追求把"新增一个 agent 要改的文件数"降到 ≤8(那需要适配器 trait 层,审计文档 Q15-2 已建议没有新 agent 需求时先不做),而是:消掉四处**会静默漂移**的手写清单,并把剩余触点登记成一份带"编译器是否能抓住"标注的清单(Task 8)。

涉及的四处手写清单(均已核实):
1. `agent_from_str` 在 `dozerd/{session_summary,summary_jobs,transcripts/mod}.rs` 各抄一份(`_ => Unknown` 兜底,漏写不报错)。
2. `transcripts/scan.rs` 的 `AGENT_ROOTS` 常量与 `discover_project_transcript_files_in` 的六元素数组。
3. `transcripts/mod.rs` 的 `delete_project_transcripts_in`(**五家,漏 v8agent**)与 `list_conversations_in`(六家 + 内层 `match` + `unreachable!`)。
4. `dozer-app/workspace/view.rs::agent_picker_spec` 的 `[(&str, PickerLaunch); 8]` 字面量数组。

### Task 4: `AgentKind::ALL` / `known()` / `from_label()`

**Files:**
- Modify: `crates/dozer-model/src/agent.rs`(`impl AgentKind` 块与文件末尾新增 `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces(后续 Task 全部依赖):
  - `AgentKind::ALL: [AgentKind; 9]`(含 `Unknown`,顺序 = Unknown, Claude, Codebuddy, Opencode, Codex, Goose, Aider, V8agent, Qodercn)
  - `AgentKind::known() -> impl Iterator<Item = AgentKind>`(`ALL` 去掉 `Unknown`,保持顺序)
  - `AgentKind::from_label(s: &str) -> Option<AgentKind>`(只认真实 agent 的 `label()`;`"shell"`、`"unknown"`、任意未知串都返回 `None`)

- [ ] **Step 1: 写失败测试**

在 `agent.rs` 末尾追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 无通配分支的穷尽 match:新增 `AgentKind` 变体时这里会**编译失败**,
    /// 逼着改动者来更新 `AgentKind::ALL`(并在下面的测试里补上新的 label)。
    fn variant_index(a: AgentKind) -> usize {
        match a {
            AgentKind::Unknown => 0,
            AgentKind::Claude => 1,
            AgentKind::Codebuddy => 2,
            AgentKind::Opencode => 3,
            AgentKind::Codex => 4,
            AgentKind::Goose => 5,
            AgentKind::Aider => 6,
            AgentKind::V8agent => 7,
            AgentKind::Qodercn => 8,
        }
    }

    #[test]
    fn all_is_listed_in_variant_order_without_gaps() {
        assert_eq!(AgentKind::ALL.len(), 9);
        for (i, a) in AgentKind::ALL.iter().enumerate() {
            assert_eq!(variant_index(*a), i, "{a:?} 在 ALL 中的位置与声明顺序不符");
        }
    }

    #[test]
    fn known_excludes_only_unknown_and_keeps_order() {
        let known: Vec<_> = AgentKind::known().collect();
        assert_eq!(known.len(), 8);
        assert!(!known.contains(&AgentKind::Unknown));
        assert_eq!(known[0], AgentKind::Claude);
        assert_eq!(known[7], AgentKind::Qodercn);
    }

    /// `label()` 是写进 SQLite 的持久化名,改动任何一个都会让旧数据读不回。
    #[test]
    fn labels_are_stable_persisted_names() {
        let expected = [
            (AgentKind::Unknown, "shell"),
            (AgentKind::Claude, "claude"),
            (AgentKind::Codebuddy, "codebuddy"),
            (AgentKind::Opencode, "opencode"),
            (AgentKind::Codex, "codex"),
            (AgentKind::Goose, "goose"),
            (AgentKind::Aider, "aider"),
            (AgentKind::V8agent, "v8agent"),
            (AgentKind::Qodercn, "qodercn"),
        ];
        for (agent, label) in expected {
            assert_eq!(agent.label(), label, "{agent:?}");
        }
    }

    #[test]
    fn from_label_round_trips_every_known_agent() {
        for a in AgentKind::known() {
            assert_eq!(AgentKind::from_label(a.label()), Some(a), "{a:?}");
        }
    }

    #[test]
    fn from_label_rejects_unknown_shell_and_garbage() {
        assert_eq!(AgentKind::from_label("shell"), None);
        assert_eq!(AgentKind::from_label("unknown"), None);
        assert_eq!(AgentKind::from_label(""), None);
        assert_eq!(AgentKind::from_label("Claude"), None, "大小写敏感,与旧 match 一致");
        assert_eq!(AgentKind::from_label("chatgpt"), None);
    }
}
```

- [ ] **Step 2: 确认失败**

Run: `cargo test -p dozer-model agent 2>&1 | tail -8`
Expected: 编译错误 `no associated item named ALL / known / from_label`

- [ ] **Step 3: 实现**

在 `impl AgentKind` 块内(`label()` 之前)加入:

```rust
    /// 全部变体(含 `Unknown`),顺序即声明顺序。新增变体必须同步这里;
    /// 测试里的无通配穷尽 `match` 会在漏改时让编译失败。
    pub const ALL: [AgentKind; 9] = [
        AgentKind::Unknown,
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Goose,
        AgentKind::Aider,
        AgentKind::V8agent,
        AgentKind::Qodercn,
    ];

    /// 真实 agent(不含 `Unknown`),保持 `ALL` 的相对顺序。
    pub fn known() -> impl Iterator<Item = AgentKind> {
        Self::ALL.into_iter().filter(|a| *a != AgentKind::Unknown)
    }

    /// `label()` 的反向查找,只认真实 agent。`None` 的含义由调用方决定
    /// (摄取/持久化读回通常 `unwrap_or(Unknown)`)。
    pub fn from_label(s: &str) -> Option<AgentKind> {
        Self::known().find(|a| a.label() == s)
    }
```

- [ ] **Step 4: 跑通**

Run: `cargo test -p dozer-model 2>&1 | grep -E "test result|FAILED"`
Expected: 全部 ok

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-model/src/agent.rs
git commit -m "feat(model): AgentKind::ALL/known/from_label as the single label table"
```

### Task 5: dozerd 三份 `agent_from_str` 收成一份

**Files:**
- Modify: `crates/dozerd/src/session_summary.rs`(`agent_from_str` 约 15–27 行;测试约 238–248 行)
- Modify: `crates/dozerd/src/summary_jobs.rs`(`agent_from_str` 约 23–35 行;测试约 730–742 行)
- Modify: `crates/dozerd/src/transcripts/mod.rs`(`agent_to_str`/`agent_from_str` 约 63–92 行;`mod tests` 追加一个测试)
- **不改** `crates/dozerd/src/group_store.rs`:它是**有意受限**的成员白名单(只支持五家入群),不是漂移的副本。

**Interfaces:**
- Consumes: `AgentKind::known()`、`AgentKind::from_label()`、`AgentKind::ALL`(Task 4)。
- Produces: 无新公开接口(三个函数都是模块私有)。

- [ ] **Step 1: 写失败/回归测试**

在 `transcripts/mod.rs` 的 `mod tests` 追加:

```rust
    /// `agent_to_str`/`agent_from_str` 是写入/读回 `conversations.agent_kind` 的编码。
    /// `Unknown` 在这里历史上写作 "unknown"(不是 `label()` 的 "shell"),必须保持。
    #[test]
    fn agent_str_round_trips_every_agent_and_keeps_unknown_spelling() {
        for a in AgentKind::ALL {
            assert_eq!(agent_from_str(agent_to_str(a)), a, "{a:?}");
        }
        assert_eq!(agent_to_str(AgentKind::Unknown), "unknown");
        assert_eq!(agent_from_str("something-new"), AgentKind::Unknown);
    }
```
并把 `session_summary.rs`、`summary_jobs.rs` 里现有往返测试的手写数组 `for agent in [AgentKind::Claude, ...]` 改为 `for agent in AgentKind::known()`(这样将来新增 agent 自动被覆盖)。

- [ ] **Step 2: 确认基线(此时应全部通过——这是重构,先钉住行为)**

Run: `cargo test -p dozerd agent_str round_trip 2>&1 | grep -E "test result|FAILED"`
Expected: ok

- [ ] **Step 3: 替换实现**

`session_summary.rs` 与 `summary_jobs.rs`:

```rust
fn agent_from_str(s: &str) -> AgentKind {
    AgentKind::from_label(s).unwrap_or(AgentKind::Unknown)
}
```
`transcripts/mod.rs`:

```rust
fn agent_to_str(a: AgentKind) -> &'static str {
    match a {
        // 历史写法,与 `label()` 的 "shell" 不同,保持以兼容已有库。
        AgentKind::Unknown => "unknown",
        other => other.label(),
    }
}

fn agent_from_str(s: &str) -> AgentKind {
    AgentKind::from_label(s).unwrap_or(AgentKind::Unknown)
}
```

- [ ] **Step 4: 跑通**

Run: `cargo test -p dozerd 2>&1 | grep -E "test result|FAILED"`
Expected: 全部 ok

- [ ] **Step 5: Commit**

```bash
git add crates/dozerd/src/session_summary.rs crates/dozerd/src/summary_jobs.rs crates/dozerd/src/transcripts/mod.rs
git commit -m "refactor(dozerd): derive agent string decoding from AgentKind::from_label"
```

### Task 6: 会话目录表 `projects_root_in` / `project_session_dir_in`,并修 v8agent 漏删

**Files:**
- Modify: `crates/dozer-core/src/agent_paths.rs`(新增两个函数 + 一个测试)
- Modify: `crates/dozerd/src/transcripts/scan.rs`(删除 `AGENT_ROOTS` 与六元素数组,约 8–21、58–75、98–111 行;更新 154 行附近回归测试注释)
- Modify: `crates/dozerd/src/transcripts/mod.rs`(`delete_project_transcripts_in` 约 825–840 行;`list_conversations_in` 约 890–930 行;`mod tests` 追加删除测试)

**Interfaces:**
- Consumes: `AgentKind::known()`、`AgentKind::ALL`(Task 4);`agent_paths` 里现有的 `*_project_dir_in(home, cwd)`。
- Produces:
  - `dozer_core::agent_paths::projects_root_in(agent: AgentKind, home: &Path) -> Option<PathBuf>`:`<home>/<root>/projects`,**不按项目**,用于启动全量扫描。`Unknown`/`Codex`/`Qodercn` 返回 `None`(Codex 按日期建目录;Qodercn 为 L1 暂无 transcript 解析)。
  - `dozer_core::agent_paths::project_session_dir_in(agent: AgentKind, home: &Path, cwd: &Path) -> Option<PathBuf>`:某项目的会话目录,`None` 的集合与上面完全一致。
  - 两个函数都是**无通配**穷尽 `match`:新增 `AgentKind` 变体时编译失败,逼着决定"有没有按项目目录"。

- [ ] **Step 1: 写失败测试**

`agent_paths.rs` 的 `mod tests` 追加(`use crate::protocol::AgentKind;` 若未引入则加上):

```rust
    #[test]
    fn project_session_dir_sits_under_projects_root_for_every_agent() {
        let home = Path::new("/h");
        let cwd = Path::new("/a/b");
        for a in AgentKind::ALL {
            let root = projects_root_in(a, home);
            let dir = project_session_dir_in(a, home, cwd);
            assert_eq!(root.is_some(), dir.is_some(), "{a:?}: 两个表的覆盖范围必须一致");
            if let (Some(r), Some(d)) = (root, dir) {
                assert_eq!(d.parent(), Some(r.as_path()), "{a:?}");
            }
        }
    }

    #[test]
    fn agents_without_per_project_dirs_are_exactly_unknown_codex_qodercn() {
        let home = Path::new("/h");
        let none: Vec<_> = AgentKind::ALL
            .into_iter()
            .filter(|a| projects_root_in(*a, home).is_none())
            .collect();
        assert_eq!(none, vec![AgentKind::Unknown, AgentKind::Codex, AgentKind::Qodercn]);
    }
```

`transcripts/mod.rs` 的 `mod tests` 追加(复用同文件已有的 `fixture(&dir, name, content)` 与 `TranscriptStore::open`,写法同 `list_conversations_includes_v8agent`):

```rust
    /// 回归:项目删除曾用手写五家清单,漏了 v8agent,导致删项目后 v8agent 会话残留。
    #[test]
    fn delete_project_transcripts_in_also_removes_v8agent_and_spares_other_projects() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let line = "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n";

        let a = std::path::Path::new("/proj/a");
        let b = std::path::Path::new("/proj/b");
        let a_dir = dozer_core::agent_paths::v8agent_project_dir_in(home.path(), a);
        let b_dir = dozer_core::agent_paths::v8agent_project_dir_in(home.path(), b);
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        let fa = fixture(&a_dir, "a.jsonl", line);
        let fb = fixture(&b_dir, "b.jsonl", line);
        store.ingest_session(AgentKind::V8agent, &fa).unwrap();
        store.ingest_session(AgentKind::V8agent, &fb).unwrap();

        let deleted = store
            .delete_project_transcripts_in(home.path(), "/proj/a")
            .unwrap();
        assert_eq!(deleted, 1);
        assert!(store
            .list_conversations_in(home.path(), "/proj/a", None, 10, 0)
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .list_conversations_in(home.path(), "/proj/b", None, 10, 0)
                .unwrap()
                .len(),
            1,
            "别的项目的 v8agent 会话不能被波及"
        );
    }
```

- [ ] **Step 2: 确认失败**

Run: `cargo test -p dozer-core agent_paths 2>&1 | tail -5` → 编译失败(函数不存在);
`cargo test -p dozerd delete_project_transcripts_in_also_removes_v8agent 2>&1 | tail -8` → 编译失败。

- [ ] **Step 3: 实现 `agent_paths`**

```rust
use crate::protocol::AgentKind;

/// 各家 agent 的"按项目建子目录"根:`<home>/<root>/projects`。供启动全量扫描枚举所有项目。
/// 无通配 `match`:新增 `AgentKind` 变体时必须在这里表态。
pub fn projects_root_in(agent: AgentKind, home: &Path) -> Option<PathBuf> {
    let root = match agent {
        AgentKind::Claude => ".claude",
        AgentKind::Codebuddy => ".codebuddy",
        AgentKind::Opencode => ".dozer/agents/opencode",
        AgentKind::Goose => ".dozer/agents/goose",
        AgentKind::Aider => ".dozer/agents/aider",
        AgentKind::V8agent => ".v8agent",
        // Codex 按日期建目录(见 `codex_sessions_dir_in`);Qodercn 目前只做到 L1,
        // 没有 transcript 解析;Unknown 不是 agent。
        AgentKind::Unknown | AgentKind::Codex | AgentKind::Qodercn => return None,
    };
    Some(home.join(root).join("projects"))
}

/// 某项目(`cwd`)在某家 agent 下的会话目录。`None` 的集合与 `projects_root_in` 一致。
/// 各家的目录编码规则不同(见 `claude_project_key` / `codebuddy_project_key`),不能统一。
pub fn project_session_dir_in(agent: AgentKind, home: &Path, cwd: &Path) -> Option<PathBuf> {
    Some(match agent {
        AgentKind::Claude => claude_project_dir_in(home, cwd),
        AgentKind::Codebuddy => codebuddy_project_dir_in(home, cwd),
        AgentKind::Opencode => opencode_project_dir_in(home, cwd),
        AgentKind::Goose => goose_project_dir_in(home, cwd),
        AgentKind::Aider => aider_project_dir_in(home, cwd),
        AgentKind::V8agent => v8agent_project_dir_in(home, cwd),
        AgentKind::Unknown | AgentKind::Codex | AgentKind::Qodercn => return None,
    })
}
```

- [ ] **Step 4: 改 `scan.rs`**

删除 `AGENT_ROOTS` 常量及 `aider_project_dir_in, claude_project_dir_in, codebuddy_project_dir_in, goose_project_dir_in, opencode_project_dir_in, v8agent_project_dir_in` 的 use(保留 `codex_sessions_dir_in, home_dir`,新增 `project_session_dir_in, projects_root_in`)。

`discover_all_transcript_files_in` 的循环改为:

```rust
    for agent in AgentKind::known() {
        let Some(projects_dir) = projects_root_in(agent, home) else {
            continue;
        };
        let Ok(rd) = std::fs::read_dir(&projects_dir) else {
            continue;
        };
        // …(循环体其余部分不变)
    }
```
`discover_project_transcript_files_in` 的数组循环改为:

```rust
    for agent in AgentKind::known() {
        let Some(dir) = project_session_dir_in(agent, home, cwd) else {
            continue;
        };
        for f in jsonl_files_in(&dir) {
            out.push((agent, f));
        }
    }
```
(`known()` 的相对顺序 Claude, Codebuddy, Opencode, Goose, Aider, V8agent 与旧数组一致,现有 `found[0]/found[1]` 断言不变。)把 154 行附近注释里的 `AGENT_ROOTS` 改述为"`projects_root_in` 的覆盖范围"。

- [ ] **Step 5: 改 `transcripts/mod.rs`**

`delete_project_transcripts_in`:删掉函数内的 `use dozer_core::agent_paths::{…}` 与五元素数组,改为:

```rust
        let cwd_path = Path::new(cwd);
        let dirs: Vec<std::path::PathBuf> = AgentKind::known()
            .filter_map(|a| dozer_core::agent_paths::project_session_dir_in(a, home, cwd_path))
            .collect();
```
`list_conversations_in`:删掉 `use` 与 `dir_agents` 数组、内层 `match … unreachable!`,改为:

```rust
        let dir_candidates: Vec<(AgentKind, String)> = AgentKind::known()
            .filter(|a| agent.is_none() || agent == Some(*a))
            .filter_map(|a| {
                dozer_core::agent_paths::project_session_dir_in(a, home, cwd_path)
                    .map(|d| (a, d.to_string_lossy().into_owned()))
            })
            .collect();
```
并更新其上方"按 `dir` 查的六家"注释为"按 `dir` 查(有按项目目录的 agent,见 `agent_paths::project_session_dir_in`)"。

- [ ] **Step 6: 跑通**

Run: `cargo test -p dozer-core -p dozerd 2>&1 | grep -E "test result|FAILED"`
Expected: 全部 ok(含新增的 v8agent 删除回归测试与 `scan.rs` 既有顺序断言)

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-core/src/agent_paths.rs crates/dozerd/src/transcripts/scan.rs crates/dozerd/src/transcripts/mod.rs
git commit -m "refactor(transcripts): table-driven agent session dirs; fix project delete missing v8agent"
```

### Task 7: Agent 选择菜单由 `AgentKind::known()` 派生

**Files:**
- Modify: `crates/dozer-app/src/workspace/view.rs`(`agent_picker_spec`,约 353–383 行)
- Modify: `crates/dozer-app/src/workspace/tests.rs`(约 860 行附近 `agent_picker_spec` 测试旁追加顺序测试)

**Interfaces:**
- Consumes: `AgentKind::known()`、`AgentKind::display_label()`(Task 4 / 现有)。
- Produces: `agent_picker_spec() -> MenuSpec<Message>` 签名不变。

- [ ] **Step 1: 写钉住顺序的测试**

在 `workspace/tests.rs` 追加:

```rust
/// 菜单项由 `AgentKind::known()` 派生后,顺序必须仍是按展示名不分大小写的字母序
/// (验收反馈 2026-08-21),shell 项在分隔线之后。
#[test]
fn agent_picker_lists_every_known_agent_in_label_order() {
    let dbg = format!("{:?}", agent_picker_spec());
    let order = ["Aider", "Claude", "CodeBuddy", "Codex", "Goose", "OpenCode", "Qoder CN", "v8agent", "Git Shell", "OS Shell"];
    let mut at = 0;
    for label in order {
        let found = dbg[at..].find(label).unwrap_or_else(|| panic!("菜单缺少或乱序: {label}"));
        at += found + label.len();
    }
    for a in AgentKind::known() {
        assert!(dbg.contains(a.display_label()), "{a:?} 不在菜单里");
    }
}
```

- [ ] **Step 2: 先跑(重构前应通过,钉住现状)**

Run: `cargo test -p dozer-app --bin dozer agent_picker 2>&1 | grep -E "test result|FAILED"`
Expected: ok

- [ ] **Step 3: 替换字面量数组**

把 `let agents: [(&str, PickerLaunch); 8] = [ … ];` 换成:

```rust
    // 由 `AgentKind::known()` 派生:新增 agent 自动出现在菜单里;按展示名不分大小写排序。
    let mut agents: Vec<(&'static str, PickerLaunch)> = AgentKind::known()
        .map(|k| (k.display_label(), PickerLaunch::Agent(Some(k))))
        .collect();
    agents.sort_by_key(|(label, _)| label.to_lowercase());
```
`agents.into_iter().map(|(label, launch)| mk(label, launch))` 无需改。

- [ ] **Step 4: 跑通**

Run: `cargo test -p dozer-app --bin dozer workspace 2>&1 | grep -E "test result|FAILED"`
Expected: ok

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/workspace/view.rs crates/dozer-app/src/workspace/tests.rs
git commit -m "refactor(workspace): derive agent picker entries from AgentKind::known"
```

### Task 8: "新增一个 agent"清单文档 + 审计文档回填

**Files:**
- Create: `docs/dozer-v2/adding-an-agent.md`
- Modify: `docs/dozer-v2/agent-pillar-audit.md`(追加 §8)

- [ ] **Step 1: 用编译器实测触点(不是凭记忆)**

在 worktree 里临时给 `AgentKind` 加一个变体 `Probe`(`dozer-model/src/agent.rs`),**不要**提交:
```bash
cargo check --workspace --all-targets 2>&1 | grep -E "^error\[E0004\]|non-exhaustive" -A2 | grep -- "-->" | sort -u
```
记录所有报错文件 = "编译器会抓住"的触点;再对 `git grep -n '_ =>' -- crates | grep -i agent` 与 `git grep -n '"qodercn"'` 的结果,人工挑出"带通配/字符串、编译器抓不住"的触点。完成后 `git checkout -- crates/dozer-model/src/agent.rs`。

- [ ] **Step 2: 写 `adding-an-agent.md`**

结构固定为三节,每个触点一行 `文件 — 作用 — 编译器能否抓住`:
1. **已由表驱动(无需手改)**:`AgentKind::ALL`/`known()`、会话目录(`agent_paths`)、字符串往返(三份 `agent_from_str`)、启动菜单。
2. **编译器会强制你表态**(无通配 match):`agent_paths::projects_root_in/project_session_dir_in`、`workspace/hook.rs::agent_dot_color/agent_icon`、`view.rs::agent_launch_command`、`transcripts/parse.rs` 两处、`headless_agent.rs` 两处,等(以 Step 1 实测为准)。
3. **编译器抓不住,必须手查**:带 `_ =>` 兜底或裸字符串的位置——`group_store.rs`(有意的白名单)、`group_chat::CREATABLE_AGENTS`、`dozer-hook/src/main.rs` 的事件名映射、`dozer-mcp/install.rs`、前端 `usage-content` / `review-trace` 的 agent 映射;并写明 qodercn 实测时踩到的两个坑(写得进读不回;无解析器却建出 0 轮空会话)。

文末给出"新增 agent 的最小步骤与验证命令"(`cargo test -p dozer-model -p dozer-core -p dozerd`)。

- [ ] **Step 3: 回填审计文档**

在 `agent-pillar-audit.md` 追加 `## 8. 第一刀落地(2026-10-16)`:写明收口了哪四处手写清单、修了 v8agent 漏删 bug、`known()` 现在驱动哪些调用点;**如实写**:新增 agent 的"必须手改文件数"实测由 N 降到 M(Step 1 的数字),未达到 ≤8,原因是适配器层(解析/hook/headless/MCP)按 Q15-2 未做。

- [ ] **Step 4: Commit**

```bash
git add docs/dozer-v2/adding-an-agent.md docs/dozer-v2/agent-pillar-audit.md
git commit -m "docs(v2): add-an-agent checklist and first-cut results"
```

---

# Part 2:三仓依赖对齐

目标状态:`digger` 的 `Cargo.toml` 里**没有任何 `[patch]`**,bytehost/byteui/dozer 全部指向已发布的 tag,且与 dozer 用同一套版本;dozer 内部同一依赖只有一个 tag。

现状实测:
- digger 的 `[patch]` 指向 `../bytehost-chrome`、`../byteui-shell`、`../dozer-store-project` 三个**已不存在**的 worktree,`cargo metadata` 直接报错——**digger 目前无法构建**。
- digger 钉 bytehost `v0.2.0`、byteui `v0.4.1`;dozer 已用 bytehost `v0.4.0`、byteui `v0.4.4`。
- dozer `main` 比 `origin/main` 多 25 个提交(含 `dozer-store-project` 拆分与 `dozer-panel-todo` 抽库),远端没有 digger 需要的新 crate。
- dozer 内部:`dozer-panel-todo` 的 byteui 是 `v0.4.3`,其余是 `v0.4.4`(本地 `[patch]` 掩盖了这一点)。
- **不动** dozer 已提交的 `Cargo.lock`:提交 `1f5f1c6b` 有意让它与本地 `.cargo/config.toml` 的 patch 保持一致;digger 解析依赖用的是它自己的 lock,不受影响。

### Task 9: 门禁脚本——同一依赖只许一个 tag(并修 panel-todo)

**Files:**
- Create: `scripts/check-shared-pins.sh`
- Modify: `crates/dozer-panel-todo/Cargo.toml`(byteui `v0.4.3` → `v0.4.4`)

**Interfaces:**
- Produces: `scripts/check-shared-pins.sh`,无参数;全部通过输出 `shared pins: ok`,否则列出冲突并 `exit 1`;`--self-test` 注入冲突必须失败。

- [ ] **Step 1: 写脚本(先写,能抓到现存的冲突)**

```bash
#!/usr/bin/env bash
# 门禁:bytehost / byteui / bytegit 在整个 workspace 里只许出现一个 tag。
# 原因:同一 git 源的两个 tag 在无本地 [patch] 的消费方(digger)里会解析成两份 crate,
# `PanelId` 之类类型互不兼容,byteui 的全局主题状态也会分裂。
# 用法: scripts/check-shared-pins.sh [--self-test]
set -euo pipefail
ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$ROOT"

collect() {
  # 输出 "<仓库名> <tag>" 去重列表
  grep -rhoE 'github.com/byteboyai/(bytehost|byteui|bytegit)", *tag *= *"[^"]+"' crates/*/Cargo.toml Cargo.toml 2>/dev/null \
    | sed -E 's#github.com/byteboyai/([a-z]+)", *tag *= *"([^"]+)"#\1 \2#' | sort -u
}

check() {
  local bad=0
  for repo in bytehost byteui bytegit; do
    local n; n=$(collect | awk -v r="$repo" '$1==r' | wc -l | tr -d ' ')
    if [ "$n" -gt 1 ]; then
      echo "CONFLICT: $repo 有 $n 个不同 tag:" >&2
      collect | awk -v r="$repo" '$1==r {print "  " $2}' >&2
      bad=1
    fi
  done
  return $bad
}

if [ "${1:-}" = "--self-test" ]; then
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/crates/a" "$tmp/crates/b"
  echo 'x = { git = "https://github.com/byteboyai/byteui", tag = "v1" }' > "$tmp/crates/a/Cargo.toml"
  echo 'x = { git = "https://github.com/byteboyai/byteui", tag = "v2" }' > "$tmp/crates/b/Cargo.toml"
  if ROOT="$tmp" bash "$0" >/dev/null 2>&1; then
    echo "self-test FAILED: 冲突没有被抓到" >&2; exit 1
  fi
  echo "self-test ok"; exit 0
fi

if check; then echo "shared pins: ok"; else exit 1; fi
```
`chmod +x scripts/check-shared-pins.sh`。

- [ ] **Step 2: 确认能抓到现存冲突**

Run: `bash scripts/check-shared-pins.sh; echo "exit=$?"`
Expected: `CONFLICT: byteui 有 2 个不同 tag: v0.4.3 v0.4.4`,`exit=1`

- [ ] **Step 3: 修 panel-todo**

`crates/dozer-panel-todo/Cargo.toml` 的 `byteui = { …, tag = "v0.4.3" }` → `"v0.4.4"`。

- [ ] **Step 4: 验证**

Run: `bash scripts/check-shared-pins.sh && bash scripts/check-shared-pins.sh --self-test && cargo build -p dozer-panel-todo 2>&1 | tail -2`
Expected: `shared pins: ok` / `self-test ok` / 编译通过(若 v0.4.3→v0.4.4 引起 todo 面板编译错误,按 byteui 变更日志修,只改 todo 面板侧)。

- [ ] **Step 5: Commit**

```bash
git add scripts/check-shared-pins.sh crates/dozer-panel-todo/Cargo.toml Cargo.lock
git commit -m "build: single byteui tag across workspace + check-shared-pins gate"
```
(`Cargo.lock` 仅当 cargo 因 todo 的 tag 变化而更新时才 add;commit 前看 `git diff --cached --stat`,不要带入无关改动。)

### Task 10: 干净解析验证 + 发布(⚠️ 需用户确认)

**Files:** 无代码改动;产出远端 tag。

- [ ] **Step 1: 在无本地 patch 的临时克隆里验证 digger 要用的四个 crate 可独立解析构建**

```bash
SCRATCH=/private/tmp/claude-501/-Users-chrischiang-Projects-CoralProjects-byteboy-dozer/b3595889-e3c5-44d2-868a-09bb0db4138f/scratchpad
rm -rf "$SCRATCH/dozer-clean" && git clone --local <worktree 路径> "$SCRATCH/dozer-clean"
cd "$SCRATCH/dozer-clean" && cargo check -p dozer-model -p dozer-services -p dozer-panel-usage -p dozer-store-project 2>&1 | tail -3
```
Expected: `Finished`。(克隆里没有 `.cargo/config.toml`,等价于 digger 的解析环境。)

- [ ] **Step 2: 合并到 main 前先确认 plan 的 Part 1/3 已审阅合并**(digger 要钉的是合并后的 main 提交)。

- [ ] **Step 3: ⚠️ 停下来,向用户确认后再推送**

推送内容 = `origin/main..main` 的 25(合并后更多)个提交,**全部**会发布到 GitHub(`byteboyai/dozer`),不仅是 digger 需要的 crate。向用户展示:
```bash
git log --oneline origin/main..main | wc -l
git diff --stat origin/main..main | tail -1
```
得到明确同意后再执行:
```bash
git push origin main
git tag libs-2026-10-16 && git push origin libs-2026-10-16
```
(tag 名对应"可供外部消费的库 crate 快照";dozer 现有 `v0.5.x` tag 是 App 发布 tag,不混用。)记录推送后的完整 sha。

### Task 11: digger 对齐(digger 仓库,分支 `chore/align-deps-2026-10`)

**Files(均在 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`):**
- Modify: `Cargo.toml`(workspace 依赖与文件末尾三个 `[patch]` 段)
- Modify: `Cargo.lock`(随构建更新)
- Modify: `crates/**` 中因 API 变化需要适配之处(**事先未知,见 Step 3**)

- [ ] **Step 1: 建分支,改依赖**

```bash
cd /Users/chrischiang/Projects/CoralProjects/byteboy/digger && git switch -c chore/align-deps-2026-10
```
`Cargo.toml` 改动:
- 七个 `bytehost-*` 的 `tag = "v0.2.0"` → `"v0.4.0"`;`bytehost-desktop` 若需要 `overlay` feature 以 digger 现有写法为准,不要擅自增删 feature。
- `byteui` 的 `tag = "v0.4.1"` → `"v0.4.4"`。
- 四个 `dozer-*` 的 `rev = "67ab0e65…"` → `tag = "libs-2026-10-16"`。
- **删除**文件末尾全部三个 `[patch.…]` 段及其"开发期临时配置"注释;把依赖段上方关于"dozer 无对应发布 tag,用 rev 钉死""开发期靠 [patch]"的注释改为现状。

- [ ] **Step 2: 解析**

Run: `cargo metadata --format-version 1 >/dev/null && echo RESOLVED`
Expected: `RESOLVED`。再确认依赖图里每个共享 crate 只有一份:
```bash
for c in byteui bytehost-core dozer-model; do echo "$c: $(cargo tree -p digger-desktop -i $c 2>/dev/null | grep -c "^$c ")"; done
```
Expected: 每个都是 `1`。若出现 2,说明还有 crate 指向不同 tag/rev,回到 Step 1 查。

- [ ] **Step 3: 构建并适配(探索性步骤)**

Run: `cargo build --workspace 2>&1 | grep -E "^error" -A6 | head -60`
bytehost 0.2→0.4 与 dozer-store-project 的 ProjectStore/MemoryStore 拆分可能带来 API 变化,**具体错误无法预知**。处理规则:
1. 只改 digger 一侧去适配已发布的 API;**不得**为迁就 digger 去改 bytehost/dozer 的已发布 API(那属于另一件事,先停下来问用户)。
2. 每修一类错误单独 commit(便于审阅),信息说明对应的上游变更。
3. 若适配量明显超出"改调用点"(例如需要重写 `ProjectHost`),停下来向用户汇报,不要自行扩大范围。

- [ ] **Step 4: 完整验证**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|FAILED"
bash scripts/check-host-deps.sh && bash scripts/check-shell-regions.sh
cargo build -p digger-desktop --release 2>&1 | tail -2
```
Expected: 全部通过。再做一次**冷启动实跑**确认不是"能编译不能跑":`scripts/run-editor-selftest.sh`(若依赖 GUI 环境无法跑则明确标 not-run 并说明)。

- [ ] **Step 5: 确认无本地 patch 残留并 commit**

Run: `grep -n "^\[patch" Cargo.toml || echo NO_PATCH`  → `NO_PATCH`
```bash
git add Cargo.toml Cargo.lock crates
git commit -m "build: align bytehost v0.4.0 / byteui v0.4.4 / dozer libs tag; drop local patches"
```

### Task 12: 版本矩阵文档 + 回填验收台账

**Files:**
- Create: `docs/dozer-v2/dependency-matrix.md`
- Modify: `docs/dozer-v2/digger-bytehost-integration/acceptance.md`(A1 行状态后追加更新说明)

- [ ] **Step 1: 写 `dependency-matrix.md`**

内容:一张表(仓库/crate × 钉的 tag 或 sha),至少含 bytehost `v0.4.0`、byteui `v0.4.4`、bytegit `v0.7.0`、dozer `libs-2026-10-16`(及其完整 sha);三条规则:①升级 bytehost/byteui 时 dozer 与 digger **同一天**一起升,否则 digger 会解析出两份 `PanelId`/主题状态;②`scripts/check-shared-pins.sh` 与 digger 的 `check-host-deps.sh` 是门禁;③dozer 的 `.cargo/config.toml` 本地 patch 只用于联调、不提交,digger 不再保留任何 `[patch]`。

- [ ] **Step 2: 回填 `acceptance.md`**

在 A1 行后(或 §4 末尾)追加一段"2026-10-16 更新":digger 现已钉 bytehost `v0.4.0`/byteui `v0.4.4`/dozer `libs-2026-10-16`,无 `[patch]`,附 Task 11 Step 4 的实际命令输出摘要(只写真实跑出来的结果,未跑的明确标 not-run)。

- [ ] **Step 3: Commit(dozer worktree)**

```bash
git add docs/dozer-v2/dependency-matrix.md docs/dozer-v2/digger-bytehost-integration/acceptance.md
git commit -m "docs(v2): dependency matrix and digger alignment record"
```

---

## 完成判据(整份 plan)

- [ ] `cargo fmt --all --check` clean;`cargo test --workspace` 无失败;`python3 -m unittest discover -s scripts/audit -p 'test_*.py'` 全绿;`check_panel_boundary.py`/`check-log-scope.sh`/`check-shared-pins.sh` 全部 ok(Part 3 + Task 9)。
- [ ] `AgentKind` 的字符串解码、会话目录、启动菜单不再有手写副本;项目删除不再漏 v8agent(Part 1)。
- [ ] digger 无 `[patch]`、能从干净环境构建并通过自身门禁;依赖图中 `byteui`/`bytehost-core`/`dozer-model` 各一份(Part 2)。
- [ ] 文档如实记录:未达到"≤8 个文件"指标及原因;任何 not-run 的验证明确标注。

## 明确不在本计划内

H8 全量动态化、Plugin Protocol、Workflow Kernel/Worktree/Delivery/Acceptance 数据模型、Agent 适配器 trait 层(解析/hook/headless/MCP)、byteui 新增 agent 专用色(Qoder 目前借用 `red`)、clippy 全部 warning 清零。
