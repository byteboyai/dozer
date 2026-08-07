# Dozer Codex/Qoder/Kilo Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `AgentKind` 扩到 Codex/Qoder/Kilo 三家，让启动器菜单能一键键入这三家 CLI，并让 Codex/Qoder 的 hook 事件能被 `dozer-hook` 正确翻译转发（`SessionInfo.agent` 能翻转成 `AgentKind::Codex`/`AgentKind::Qoder`）；Kilo 因为可扩展机制存在实质性不确定性（见下），本计划只交付一份 spike 决策记录，实际插件代码留给独立后续计划。

**Architecture:** 沿用 `docs/superpowers/plans/2026-07-31-dozer-multi-agent-foundation.md`（协议层 `AgentKind`）与 `2026-07-31-dozer-codebuddy-adapter.md`（hook 家族 adapter 的 spike→翻译表→安装器三段式）已经跑通的骨架。Codex/Qoder 的 hook payload 字段名、hook 注册 JSON 结构均已通过官方文档核实与 Claude/CodeBuddy 逐字对齐（见 Global Constraints），因此翻译表与安装器可以直接实现，只把"事件名词汇是否与文档完全一致""transcript 精确 schema"这两项交给各自的 spike 任务用真实 CLI 核实。Kilo 没有进程级 hook，只有 TS/JS 插件；但 Kilo 官方一个 issue（`Kilo-Org/kilocode#5827`）把"暴露 session 生命周期钩子给第三方"标记为"Closed as not planned"，与官方插件文档里描述的 `event` hook（`session.*`/`message.*`/`tool.*`）存在潜在矛盾，真实可行性未知——本计划把 Kilo 的 spike 结果当作"go/no-go 决策"，不预先承诺实现。

**Tech Stack:** Rust 2024（`dozer-core`、`dozer-hook`、`dozer-app`）；Kilo 的 spike 涉及 TypeScript/Node 生态但本计划不产出 TS 代码。

## Global Constraints

- **前置依赖**：本计划假定 `2026-07-31-dozer-multi-agent-foundation.md` 与 `2026-07-31-dozer-codebuddy-adapter.md` 已合并——`AgentKind`（当前 4 个变体）、`dozer-hook <agent> <event>` CLI 形态、`install.rs::run_at`/`settings_path_for` 的按-agent-分派机制、`dozer-app` 里"按 `AgentKind` 分派"的 `parse_transcript`/`conversation_title`/`parse_usage` 都已存在。本计划所有"Modify"的行号以这两份计划落地后、且截至 2026-08-07 的 `main` 分支文件状态为准（已核对）。
- 参照 spec：`docs/superpowers/specs/2026-08-07-dozer-multi-agent-codex-qoder-kilo-design.md`。
- **Codex hook 注册 JSON 结构、Qoder hook 注册 JSON 结构均已通过官方文档核实**：两者都是 `{"hooks": {"EventName": [{"matcher": "...", "hooks": [{"type": "command", "command": "..."}]}]}}` 这个三层结构，与 Claude Code 逐字对齐（`matcher` 字段可选，省略即匹配全部——`install.rs::run_at` 现有实现本来就不写 `matcher`，对 Claude/CodeBuddy 已验证有效）。本计划据此直接实现安装器，不再单独为"JSON 结构是否兼容"开 spike 分支；两家的 spike 任务只核实"事件名/字段名是否与文档一致"和"transcript 精确 schema"。
- **本计划不包含** Codex/Qoder 的真实 transcript（JSONL）解析与历史对话目录扫描——`transcript.rs` 的 `Codex`/`Qoder` 分支本计划实现为返回空结果，`conversation.rs::list_all_conversations` 不纳入这两家的目录扫描。原因：Codex 的会话落盘经现场核实（`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`）**不是**按 cwd 建目录的（与 Claude/CodeBuddy/OpenCode 的 `<root>/projects/<cwd-key>/` 结构完全不同，需要扫全量文件按内容里的 cwd 字段过滤），在没有 spike 核实 Qoder 是否同构之前，写一个"看起来合理"的目录路径函数是誤导性的诚实降级都谈不上——不写比写错更诚实。这部分留给独立后续计划，做法与当年 CodeBuddy transcript 解析器拆成 `2026-08-05-dozer-codebuddy-transcript-parser.md` 独立计划同一个理由。
- **本计划不包含** Kilo 的插件代码（`dozer.ts`/`dozer-translate.ts`/`kilo_install.rs`）——Task 10 的 spike 结果如果证实可行，实现放到独立后续计划；如果证实不可行，Kilo 在 `agent_dot_color`/`agent_icon` 等处的"诚实降级"分支（本计划 Task 2/3 已经加好）就是最终状态，不再有后续工作。
- 事件名翻译表以 spec §5/§6 为准，不臆造；`AgentKind::label()` 的 CLI 名字符串（`"codex"`/`"qoder"`/`"kilo"`）已核实是三家官方 CLI 的真实调用命令（Codex 本机已装，`codex --version` 可用；Qoder 命令名经 Zed ACP 官方文档核实为 `qoder`；Kilo 命令名经 `@kilocode/cli` 官方 npm 包 `bin` 字段核实为 `kilo`）。
- 所有新增/修改的测试遵循仓库既有风格：纯函数表驱动测试，不新增 iced widget 快照测试。

---

## Task 1: `AgentKind` 扩到 Codex/Qoder/Kilo

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs:17-25`（`AgentKind` 枚举）
- Modify: `crates/dozer-core/src/protocol.rs:27-37`（`label()`）
- Modify: `crates/dozer-core/src/protocol.rs:415-442`（既有 `agent_kind_*` 测试）

**Interfaces:**
- Produces: `AgentKind::{Codex, Qoder, Kilo}` 三个新变体（`#[serde(rename_all = "snake_case")]` 下分别序列化为 `"codex"`/`"qoder"`/`"kilo"`）；`label()` 对应返回 `"codex"`/`"qoder"`/`"kilo"`。

- [ ] **Step 1: 写失败的测试**

替换 `protocol.rs` 第 420-442 行的两个测试（在既有断言基础上追加新变体，不删旧断言）：

```rust
    #[test]
    fn agent_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&AgentKind::Codebuddy).unwrap(),
            "\"codebuddy\""
        );
        assert_eq!(
            serde_json::to_string(&AgentKind::Opencode).unwrap(),
            "\"opencode\""
        );
        assert_eq!(
            serde_json::to_string(&AgentKind::Codex).unwrap(),
            "\"codex\""
        );
        assert_eq!(
            serde_json::to_string(&AgentKind::Qoder).unwrap(),
            "\"qoder\""
        );
        assert_eq!(
            serde_json::to_string(&AgentKind::Kilo).unwrap(),
            "\"kilo\""
        );
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"claude\"").unwrap(),
            AgentKind::Claude
        );
    }

    #[test]
    fn agent_kind_label_matches_variant() {
        assert_eq!(AgentKind::Unknown.label(), "未知");
        assert_eq!(AgentKind::Claude.label(), "claude");
        assert_eq!(AgentKind::Codebuddy.label(), "codebuddy");
        assert_eq!(AgentKind::Opencode.label(), "opencode");
        assert_eq!(AgentKind::Codex.label(), "codex");
        assert_eq!(AgentKind::Qoder.label(), "qoder");
        assert_eq!(AgentKind::Kilo.label(), "kilo");
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core agent_kind -- --nocapture`
Expected: FAIL，`AgentKind::Codex`/`Qoder`/`Kilo` 未定义，编译错误。

- [ ] **Step 3: 实现**

`protocol.rs` 第 17-25 行的枚举：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Unknown,
    Claude,
    Codebuddy,
    Opencode,
    Codex,
    Qoder,
    Kilo,
}
```

第 27-37 行的 `label()`：

```rust
impl AgentKind {
    /// 展示用短标签（对话历史副行、GUI 角标）。同时也是启动器菜单键入
    /// 的 CLI 命令名——三家均已核实与官方命令名一致（见计划 Global
    /// Constraints）。
    pub fn label(&self) -> &'static str {
        match self {
            AgentKind::Unknown => "未知",
            AgentKind::Claude => "claude",
            AgentKind::Codebuddy => "codebuddy",
            AgentKind::Opencode => "opencode",
            AgentKind::Codex => "codex",
            AgentKind::Qoder => "qoder",
            AgentKind::Kilo => "kilo",
        }
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core -- --nocapture`
Expected: `dozer-core` 全绿（`dozerd`/`dozer-app`/`dozer-hook` 此刻应该编译失败，属预期，后续任务修复）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): extend AgentKind with Codex/Qoder/Kilo"
```

---

## Task 2: `dozer-app` 消费侧诚实降级——补齐三个新变体的穷举分支

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs:172-187`（`parse_transcript`）
- Modify: `crates/dozer-app/src/conversation.rs:53-65`（`conversation_title`）
- Modify: `crates/dozer-app/src/usage.rs:36-46`（`parse_usage`）
- Modify: `crates/dozer-app/src/usage.rs:253-258`（`daily_totals_by_agent` 内部 `match meta.agent`）

**Interfaces:**
- Consumes: `AgentKind::{Codex, Qoder, Kilo}`（Task 1）。
- Produces：三个新变体在所有既有穷举 match 处都有明确、诚实、可测试的行为：`Kilo` 复用 Claude-shaped 分支（因为 Kilo 的 transcript 落盘将来由 `dozer-hook` 代写成 Claude 形状，跟 OpenCode 同理，不依赖 Kilo 内部实现——见 spec §7）；`Codex`/`Qoder` 返回空结果（真实 schema 待各自 spike 产出 fixture 后另开计划补齐，跟 CodeBuddy 当年同一个降级理由）。

- [ ] **Step 1: 写失败的测试**

`transcript.rs` 现有测试模块（`opencode_reuses_claude_shaped_parser`/`codebuddy_and_unknown_yield_empty_until_schema_confirmed` 附近）追加：

```rust
    #[test]
    fn kilo_reuses_claude_shaped_parser() {
        // Kilo 的 transcript 由 dozer-hook 代写成 Claude 形状（跟 OpenCode
        // 同理，见 spec §7），不依赖 Kilo 插件本身是否已实现。
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"kilo 里也这么解析"}}"#;
        let entries = parse_transcript(AgentKind::Kilo, jsonl);
        assert_eq!(
            entries,
            vec![ReviewEntry::Human {
                text: "kilo 里也这么解析".into()
            }]
        );
    }

    #[test]
    fn codex_and_qoder_yield_empty_until_schema_confirmed() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"应该被忽略"}}"#;
        assert!(parse_transcript(AgentKind::Codex, jsonl).is_empty());
        assert!(parse_transcript(AgentKind::Qoder, jsonl).is_empty());
    }
```

`conversation.rs` 现有测试模块追加：

```rust
    #[test]
    fn codex_and_qoder_title_is_none_until_schema_confirmed() {
        let head = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"忽略\"}}\n";
        assert_eq!(conversation_title(AgentKind::Codex, head), None);
        assert_eq!(conversation_title(AgentKind::Qoder, head), None);
    }

    #[test]
    fn kilo_title_reuses_claude_shaped_parser() {
        let head = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n";
        assert_eq!(
            conversation_title(AgentKind::Kilo, head).as_deref(),
            Some("改一下 README")
        );
    }
```

`usage.rs` 现有测试模块（`parse_usage` 相关测试附近，约第 770 行）追加：

```rust
    #[test]
    fn kilo_usage_reuses_claude_shaped_parser() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        assert_eq!(parse_usage(AgentKind::Kilo, jsonl).turns, 1);
    }

    #[test]
    fn codex_and_qoder_usage_is_default_until_schema_confirmed() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"hi"}}"#;
        assert_eq!(parse_usage(AgentKind::Codex, jsonl), ConversationUsage::default());
        assert_eq!(parse_usage(AgentKind::Qoder, jsonl), ConversationUsage::default());
    }
```

`usage.rs` 的 `daily_totals_by_agent` 测试（约第 878 行 `daily_totals_by_agent_buckets_by_day_and_sums_per_agent` 附近）追加：

```rust
    #[test]
    fn daily_totals_ignores_agents_without_dedicated_bucket() {
        // Codex/Qoder/Kilo 目前没有专属的 DayAgentTotals 字段（这三家的
        // 用量还进不了统计，见计划 Global Constraints），跟 Unknown 一样
        // 被忽略，不能 panic。
        let rows = vec![(
            meta_at(AgentKind::Codex, 0),
            usage_with_tokens(99),
        )];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].claude + days[0].codebuddy + days[0].opencode, 0);
    }
```

（`meta_at`/`usage_with_tokens` 是 `usage.rs` 测试模块里已经存在的辅助函数，直接复用，不新增。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo build -p dozer-app`
Expected: FAIL，四处 `match agent { ... }` 均缺 `Codex`/`Qoder`/`Kilo` 分支，编译错误（"non-exhaustive patterns"）。

- [ ] **Step 3: 实现**

`transcript.rs` 第 172-187 行：

```rust
/// 按 agent 分派 transcript 解析。`Opencode`/`Kilo` 复用 Claude 分支——
/// dozer-hook 代写它们的 transcript 时就是按 Claude 字段形状写的（spec
/// §5.3/§7），不是巧合，且不依赖各自适配层是否已实现。`Unknown` 也复用
/// Claude 分支：老装的 hook 上报的事件 agent 字段恒 `Unknown`，磁盘上
/// 现存的 transcript 事实上全是 Claude 形状。`Codebuddy` 走独立 schema
/// 的解析器。`Codex`/`Qoder` 暂时返回空：真实 transcript schema 待各自
/// spike 产出 fixture 后另开计划接（见本计划 Global Constraints），在
/// 那之前"不产出数据"是唯一诚实的行为，不是占位符。
pub fn parse_transcript(agent: AgentKind, jsonl: &str) -> Vec<ReviewEntry> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            parse_claude_shaped_jsonl(jsonl)
        }
        AgentKind::Codebuddy => parse_codebuddy_shaped_jsonl(jsonl),
        AgentKind::Codex | AgentKind::Qoder => Vec::new(),
    }
}
```

`conversation.rs` 第 53-65 行：

```rust
/// transcript 首段 → 首句人类发言。按 agent 分派——Claude/Opencode/Kilo/
/// Unknown 共用一套 schema（`type:"user"` + `message.content` 是字符串)，
/// CodeBuddy 是独立 schema。`Codex`/`Qoder` 暂时返回 `None`（真实 schema
/// 待 spike 确认；这两家目前也不会被 `list_all_conversations` 扫到，本
/// 分支纯粹是让穷举 match 编译通过，见计划 Global Constraints）。纯函数。
pub fn conversation_title(agent: AgentKind, jsonl_head: &str) -> Option<String> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            claude_shaped_title(jsonl_head)
        }
        AgentKind::Codebuddy => codebuddy_shaped_title(jsonl_head),
        AgentKind::Codex | AgentKind::Qoder => None,
    }
}
```

`usage.rs` 第 36-46 行：

```rust
/// 按 agent 分派解析,Claude/OpenCode/Kilo 共用一套 schema、CodeBuddy 独立
/// 一套,与 `transcript.rs::parse_transcript` 同一分派方式；`Codex`/`Qoder`
/// 暂时返回默认值(全零),理由同 `parse_transcript`。单行解析失败/字段
/// 缺失一律跳过该行/记 0,不 panic、不中断整份文件的解析。
pub fn parse_usage(agent: AgentKind, jsonl: &str) -> ConversationUsage {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            parse_claude_shaped_usage(jsonl)
        }
        AgentKind::Codebuddy => parse_codebuddy_shaped_usage(jsonl),
        AgentKind::Codex | AgentKind::Qoder => ConversationUsage::default(),
    }
}
```

`usage.rs` 第 253-258 行：

```rust
        match meta.agent {
            AgentKind::Claude => entry.0 += total,
            AgentKind::Codebuddy => entry.1 += total,
            AgentKind::Opencode => entry.2 += total,
            AgentKind::Unknown | AgentKind::Codex | AgentKind::Qoder | AgentKind::Kilo => {}
        }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo build -p dozer-app`
Expected: 编译通过（此刻 `agent_dot_color`/`agent_icon` 等 workspace.rs 里的穷举 match 尚未补齐，仍会编译失败——这是预期，Task 3 会修）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/transcript.rs crates/dozer-app/src/conversation.rs crates/dozer-app/src/usage.rs
git commit -m "feat(dozer-app): honest degradation for Codex/Qoder/Kilo in transcript/usage/conversation dispatch"
```

---

## Task 3: 启动器菜单——新增强调色 + Codex/Qoder/Kilo 三项

**Files:**
- Modify: `crates/dozer-app/src/theme.rs:22`（`RED` 之后新增三个颜色常量）
- Modify: `crates/dozer-app/src/workspace.rs:9255-9262`（`agent_dot_color`）
- Modify: `crates/dozer-app/src/workspace.rs:9266-9273`（`agent_icon`）
- Modify: `crates/dozer-app/src/workspace.rs:6456-6462`（`agent_picker_popup` 菜单项数组）
- Modify: `crates/dozer-app/src/workspace.rs:10840-10897`（既有 `agent_dot_color_maps_each_kind_and_avoids_gold`/`agent_icon_maps_each_kind_to_brand_icon`/`agent_cli_command_maps_known_agents_and_none_for_unknown`/`picker_launch_command_maps_selection_to_initial_command` 四个测试）

**Interfaces:**
- Consumes: `AgentKind::{Codex, Qoder, Kilo}`（Task 1）、`theme::{ORANGE, MAGENTA, BLUE}`（本任务新增）。
- Produces: `agent_dot_color`/`agent_icon` 对三个新变体的完整实现；`agent_picker_popup` 菜单从 5 项扩到 8 项；`agent_cli_command`（既有实现 `known => Some(known.label())` 已经泛化，本任务**不改这个函数本身**，只扩测试断言）。

**说明——`agent_cli_command` 为什么不用改代码**：其现有实现是

```rust
fn agent_cli_command(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Unknown => None,
        known => Some(known.label()),
    }
}
```

`known => Some(known.label())` 这一支已经泛化覆盖所有非 `Unknown` 变体，Task 1 给 `label()` 加的 `"codex"`/`"qoder"`/`"kilo"` 会自动生效，不需要在这里加分支——只需要扩测试断言证明它确实生效（本任务 Step 1 已包含）。

**关于图标——为什么用 `IconKind::Bot` 兜底而不新增品牌图标**：spec §8/§6 明确允许"找不到合适素材时回落到 `IconKind::Bot`"。Codex/Qoder/Kilo 三家目前都没有确认可用的品牌 SVG 来源（不像 Claude/CodeBuddy 取自 Simple Icons、OpenCode 取自官网 favicon 那样有明确先例可循），本计划不臆造/不代拿其他产品的 logo，直接用现有 `Bot` 通用图标——这是一个真实、经过测试的决定，不是占位符，后续若找到合适素材可以在不改架构的情况下单独换图标。

- [ ] **Step 1: 写失败的测试**

完整替换 `workspace.rs` 第 10840-10897 行这四个测试：

```rust
    #[test]
    fn agent_dot_color_maps_each_kind_and_avoids_gold() {
        let cases = [
            (AgentKind::Claude, theme::CYAN),
            (AgentKind::Codebuddy, theme::PURPLE),
            (AgentKind::Opencode, theme::GREEN),
            (AgentKind::Codex, theme::ORANGE),
            (AgentKind::Qoder, theme::MAGENTA),
            (AgentKind::Kilo, theme::BLUE),
            (AgentKind::Unknown, theme::DIM),
        ];
        for (agent, expected) in cases {
            let color = agent_dot_color(agent);
            assert_eq!(color, expected, "{agent:?}");
            assert_ne!(
                color,
                theme::GOLD,
                "{agent:?}: agent 圆点不得使用甲方动作专属的金色"
            );
        }
    }

    #[test]
    fn agent_icon_maps_each_kind_to_brand_icon() {
        assert_eq!(agent_icon(AgentKind::Claude), IconKind::Claude);
        assert_eq!(agent_icon(AgentKind::Codebuddy), IconKind::Codebuddy);
        assert_eq!(agent_icon(AgentKind::Opencode), IconKind::Opencode);
        // Codex/Qoder/Kilo 暂无确认可用的品牌素材，回落通用 Bot 图标
        // （见计划 Task 3 说明，非占位符——spec §8/§6 明确允许的兜底）。
        assert_eq!(agent_icon(AgentKind::Codex), IconKind::Bot);
        assert_eq!(agent_icon(AgentKind::Qoder), IconKind::Bot);
        assert_eq!(agent_icon(AgentKind::Kilo), IconKind::Bot);
        // Unknown 同样回落 Bot 图标。
        assert_eq!(agent_icon(AgentKind::Unknown), IconKind::Bot);
    }

    #[test]
    fn agent_cli_command_maps_known_agents_and_none_for_unknown() {
        assert_eq!(agent_cli_command(AgentKind::Claude), Some("claude"));
        assert_eq!(agent_cli_command(AgentKind::Codebuddy), Some("codebuddy"));
        assert_eq!(agent_cli_command(AgentKind::Opencode), Some("opencode"));
        assert_eq!(agent_cli_command(AgentKind::Codex), Some("codex"));
        assert_eq!(agent_cli_command(AgentKind::Qoder), Some("qoder"));
        assert_eq!(agent_cli_command(AgentKind::Kilo), Some("kilo"));
        assert_eq!(agent_cli_command(AgentKind::Unknown), None);
    }

    #[test]
    fn picker_launch_command_maps_selection_to_initial_command() {
        // 已知 agent → 其 CLI 名(复用 agent_cli_command)。
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Claude))),
            Some("claude".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
            Some("codebuddy".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Opencode))),
            Some("opencode".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Codex))),
            Some("codex".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Qoder))),
            Some("qoder".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Kilo))),
            Some("kilo".to_string())
        );
        // 纯 Shell → 不键入任何初始命令。
        assert_eq!(picker_launch_command(PickerLaunch::Agent(None)), None);
        // Git Shell → 项目根开 shell 后自动跑 git status。
        assert_eq!(
            picker_launch_command(PickerLaunch::Git),
            Some("git status".to_string())
        );
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo build -p dozer-app`
Expected: FAIL，`theme::ORANGE`/`theme::MAGENTA`/`theme::BLUE` 未定义 + `agent_dot_color`/`agent_icon` 穷举 match 缺分支，编译错误。

- [ ] **Step 3: 实现**

`theme.rs` 第 22 行（`RED` 常量）之后新增：

```rust
/// Codex 强调色（picker 圆点/图标底色）。ByteBoy2077 核心 5 色（bg/金/
/// 奶油/青/绿）之外的扩展色，`PURPLE` 已是先例。
pub const ORANGE: Color = c(0xFF, 0x9B, 0x4D);
pub const MAGENTA: Color = c(0xFF, 0x6E, 0xC7);
pub const BLUE: Color = c(0x4D, 0x8C, 0xFF);
```

`workspace.rs` 第 9255-9262 行：

```rust
pub(crate) fn agent_dot_color(agent: AgentKind) -> Color {
    match agent {
        AgentKind::Claude => theme::CYAN,
        AgentKind::Codebuddy => theme::PURPLE,
        AgentKind::Opencode => theme::GREEN,
        AgentKind::Codex => theme::ORANGE,
        AgentKind::Qoder => theme::MAGENTA,
        AgentKind::Kilo => theme::BLUE,
        AgentKind::Unknown => theme::DIM,
    }
}
```

`workspace.rs` 第 9266-9273 行：

```rust
fn agent_icon(agent: AgentKind) -> IconKind {
    match agent {
        AgentKind::Claude => IconKind::Claude,
        AgentKind::Codebuddy => IconKind::Codebuddy,
        AgentKind::Opencode => IconKind::Opencode,
        // 暂无确认可用的品牌素材，回落通用图标（spec §8/§6 明确允许）。
        AgentKind::Codex | AgentKind::Qoder | AgentKind::Kilo | AgentKind::Unknown => {
            IconKind::Bot
        }
    }
}
```

`workspace.rs` 第 6456-6462 行的菜单项数组：

```rust
    let items: [(&str, PickerLaunch); 8] = [
        ("Claude", PickerLaunch::Agent(Some(AgentKind::Claude))),
        ("CodeBuddy", PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
        ("OpenCode", PickerLaunch::Agent(Some(AgentKind::Opencode))),
        ("Codex", PickerLaunch::Agent(Some(AgentKind::Codex))),
        ("Qoder", PickerLaunch::Agent(Some(AgentKind::Qoder))),
        ("Kilo", PickerLaunch::Agent(Some(AgentKind::Kilo))),
        ("纯 Shell", PickerLaunch::Agent(None)),
        ("Git Shell", PickerLaunch::Git),
    ];
```

同一函数doc注释（紧邻数组上方，原文"五个选项 Claude/CodeBuddy/OpenCode/纯 Shell/Git Shell"）改成"八个选项 Claude/CodeBuddy/OpenCode/Codex/Qoder/Kilo/纯 Shell/Git Shell"。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app -- --nocapture`
Expected: PASS，全量 `dozer-app` 测试绿（含 Task 2 新增测试，此刻整个 crate 首次重新可编译）。

- [ ] **Step 5: 全 workspace 编译确认无回归**

Run: `cargo build --workspace`
Expected: 全部 crate 编译通过（`dozer-hook`/`dozerd` 此刻本来就没有引用新枚举值的穷举 match，不受影响）。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-app/src/theme.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): launcher menu gains Codex/Qoder/Kilo entries with dedicated accent colors"
```

---

## Task 4: Spike——验证 Codex hook 注册与 transcript 落盘

这是人工验证任务，不是写代码任务；产出一份决策记录，供 Task 5/6 与未来的 Codex transcript 解析计划使用。Codex CLI 在本机已装好（`codex --version` → `codex-cli 0.146.0`），不需要额外安装。

**Files:**
- Create: `crates/dozer-hook/fixtures/codex-transcript-sample.jsonl`（若拿到可脱敏样本）
- Create: `docs/superpowers/specs/2026-08-07-codex-spike-findings.md`

- [ ] **Step 1: 确认 Codex CLI 可用**

Run: `codex --version`
Expected: 打印版本号（已确认可用：`codex-cli 0.146.0`）。

- [ ] **Step 2: 备份现有配置**

```bash
cp -n ~/.codex/hooks.json ~/.codex/hooks.json.dozer-spike-backup 2>/dev/null || true
```

（若 `~/.codex/hooks.json` 原本不存在，不需要备份，跳过即可。）

- [ ] **Step 3: 写探针 hooks.json，跑一次非交互会话**

写 `~/.codex/hooks.json`：

```json
{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===SessionStart===\\n\" >> /tmp/codex-hook-probe.log; cat >> /tmp/codex-hook-probe.log'" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===UserPromptSubmit===\\n\" >> /tmp/codex-hook-probe.log; cat >> /tmp/codex-hook-probe.log'" }] }],
    "PreToolUse": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===PreToolUse===\\n\" >> /tmp/codex-hook-probe.log; cat >> /tmp/codex-hook-probe.log'" }] }],
    "PostToolUse": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===PostToolUse===\\n\" >> /tmp/codex-hook-probe.log; cat >> /tmp/codex-hook-probe.log'" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===Stop===\\n\" >> /tmp/codex-hook-probe.log; cat >> /tmp/codex-hook-probe.log'" }] }],
    "SessionEnd": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===SessionEnd===\\n\" >> /tmp/codex-hook-probe.log; cat >> /tmp/codex-hook-probe.log'" }] }]
  }
}
```

跑：

```bash
cd /tmp && rm -f /tmp/codex-hook-probe.log && codex exec "reply with exactly one word: hello"
```

Expected 两种可能之一：
- **命中**：`/tmp/codex-hook-probe.log` 有内容，说明本计划 Global Constraints 里假设的 JSON 结构可行，走 Task 5/6 现有设计。
- **未命中**：文件为空，说明结构或注册路径需要调整——记录实际观察到的行为，Task 5/6 的实现需要按实测结果调整（不在本任务范围，写清楚交给下一个任务处理）。

- [ ] **Step 4: 检查探测日志里的字段**

```bash
cat /tmp/codex-hook-probe.log
```

确认每个事件段是否含 `session_id`/`transcript_path`/`cwd`/`hook_event_name`/`tool_name`/`tool_input` 字段（本计划假设值，见 Global Constraints）。记下实际字段名——如果跟假设不一致，Task 5 的翻译表要跟着改。

- [ ] **Step 5: 抓一份真实 transcript 样本（若 `transcript_path` 非空）**

```bash
cat /tmp/codex-hook-probe.log | grep -A2 transcript_path
```

若能定位到真实 transcript 文件，复制一份出来脱敏（删掉真实项目路径/文件内容，只留结构），存到 `crates/dozer-hook/fixtures/codex-transcript-sample.jsonl`。这份 fixture 是未来"Codex transcript 解析"计划的输入，不是本计划要解析的对象。

- [ ] **Step 6: 还原配置**

```bash
if [ -f ~/.codex/hooks.json.dozer-spike-backup ]; then
  mv ~/.codex/hooks.json.dozer-spike-backup ~/.codex/hooks.json
else
  rm -f ~/.codex/hooks.json
fi
rm -f /tmp/codex-hook-probe.log
```

- [ ] **Step 7: 写决策记录**

创建 `docs/superpowers/specs/2026-08-07-codex-spike-findings.md`，内容至少包含：hooks.json 结构实测是否与假设一致、探测日志里的实际字段名（跟假设的差异，如果有）、transcript 样本是否拿到、对 Task 5/6 的影响（照原计划走，还是需要调整）。格式参照 `docs/superpowers/specs/2026-07-31-codebuddy-spike-findings.md`（结论一览表 + 验证步骤 + 对后续计划的影响）。

- [ ] **Step 8: 提交**

```bash
git add docs/superpowers/specs/2026-08-07-codex-spike-findings.md
# 若 Step 5 拿到样本，一并加入：
git add crates/dozer-hook/fixtures/codex-transcript-sample.jsonl 2>/dev/null || true
git commit -m "docs: record Codex hook registration spike findings"
```

---

## Task 5: Codex 事件名翻译表 + `forward()` 接入

**Files:**
- Create: `crates/dozer-hook/src/codex.rs`
- Modify: `crates/dozer-hook/src/main.rs`（顶部 `mod` 声明、`parse_agent`、`resolve_event`）

**Interfaces:**
- Consumes: `AgentKind::Codex`（Task 1）；Task 4 spike 结论（若翻译表需要调整，以 spike 决策记录为准，本任务实现的是 spec §5 的默认假设）。
- Produces: `codex::translate_event(raw: &str) -> Option<String>`；`parse_agent("codex") == AgentKind::Codex`；`resolve_event(AgentKind::Codex, ..)` 套用该翻译表。

- [ ] **Step 1: 写失败的测试**

创建 `crates/dozer-hook/src/codex.rs`：

```rust
//! Codex 原生 hook 事件名 → dozerd 规范事件名翻译（spec §5）。规范词汇表
//! 就是 dozerd::agent_state_for 认识的 7 个 Claude 事件名，这里只做翻译，
//! 不引入新词汇。事件名词汇经官方文档核实（`developers.openai.com/codex/
//! hooks`），与 Claude Code 高度对齐；Task 4 spike 若发现实测偏差，回来
//! 改这张表即可，不影响其余逻辑。

/// `None` = 这个事件不转发给 dozerd（压缩/子 agent 生命周期，不影响顶层
/// 四态机）。未知事件原样透传——翻译层不对"没见过的事件"做任何假设，
/// 交给 dozerd 那边的 `agent_state_for` 自己因为认不出而不改状态。
pub fn translate_event(raw: &str) -> Option<String> {
    match raw {
        "SessionStart" => Some("SessionStart"),
        "UserPromptSubmit" => Some("UserPromptSubmit"),
        "PreToolUse" => Some("PreToolUse"),
        "PostToolUse" => Some("PostToolUse"),
        "PermissionRequest" => Some("Notification"),
        "Stop" => Some("Stop"),
        "SessionEnd" => Some("SessionEnd"),
        "PreCompact" | "PostCompact" | "SubagentStart" | "SubagentStop" => return None,
        other => other,
    }
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_events_map_to_same_name() {
        assert_eq!(translate_event("SessionStart"), Some("SessionStart".to_string()));
        assert_eq!(translate_event("UserPromptSubmit"), Some("UserPromptSubmit".to_string()));
        assert_eq!(translate_event("PreToolUse"), Some("PreToolUse".to_string()));
        assert_eq!(translate_event("PostToolUse"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("Stop"), Some("Stop".to_string()));
        assert_eq!(translate_event("SessionEnd"), Some("SessionEnd".to_string()));
    }

    #[test]
    fn permission_request_merges_into_notification() {
        assert_eq!(translate_event("PermissionRequest"), Some("Notification".to_string()));
    }

    #[test]
    fn compaction_and_subagent_events_are_dropped() {
        assert_eq!(translate_event("PreCompact"), None);
        assert_eq!(translate_event("PostCompact"), None);
        assert_eq!(translate_event("SubagentStart"), None);
        assert_eq!(translate_event("SubagentStop"), None);
    }

    #[test]
    fn unknown_event_passes_through_unmapped() {
        assert_eq!(translate_event("SomeFutureEvent"), Some("SomeFutureEvent".to_string()));
    }
}
```

`main.rs` 顶部 `mod` 声明（现有 `mod codebuddy;` 之后）加：

```rust
mod codex;
```

`main.rs` 的 `parse_agent` 函数追加一行：

```rust
fn parse_agent(arg: &str) -> AgentKind {
    match arg {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        "codex" => AgentKind::Codex,
        _ => AgentKind::Unknown,
    }
}
```

`main.rs` 的 `resolve_event` 函数追加一支：

```rust
fn resolve_event(agent: AgentKind, event_arg: Option<&str>) -> Option<String> {
    let raw = event_arg.map(str::to_string);
    let raw = raw.unwrap_or_else(|| "unknown".to_string());
    match agent {
        AgentKind::Codebuddy => codebuddy::translate_event(&raw),
        AgentKind::Codex => codex::translate_event(&raw),
        _ => Some(raw),
    }
}
```

`main.rs` 现有 `#[cfg(test)] mod tests` 追加：

```rust
    #[test]
    fn parse_agent_recognizes_codex() {
        assert_eq!(parse_agent("codex"), AgentKind::Codex);
    }

    #[test]
    fn resolve_event_translates_codex_permission_request() {
        assert_eq!(
            resolve_event(AgentKind::Codex, Some("PermissionRequest")),
            Some("Notification".to_string())
        );
    }

    #[test]
    fn resolve_event_drops_codex_compaction_events() {
        assert_eq!(resolve_event(AgentKind::Codex, Some("PreCompact")), None);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook codex -- --nocapture`
Expected: FAIL，`crates/dozer-hook/src/codex.rs` 不存在/`parse_agent`/`resolve_event` 未识别 `codex`，编译错误。

- [ ] **Step 3: 实现**

已在 Step 1 给出全部实现代码，直接落地。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS（新增 7 个测试 + 既有测试全绿）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-hook/src/codex.rs crates/dozer-hook/src/main.rs
git commit -m "feat(dozer-hook): Codex native event name translation table"
```

---

## Task 6: Codex hook 安装器

**Files:**
- Modify: `crates/dozer-hook/src/install.rs:21-38`（`settings_path_for`）
- Modify: `crates/dozer-hook/src/install.rs`（既有测试模块追加 Codex 用例）

**Interfaces:**
- Consumes: `install::run_at`（已存在、按 agent 泛化，本任务不改其实现）。
- Produces: `settings_path_for("codex") -> ~/.codex/hooks.json`（`DOZER_CODEX_SETTINGS` 环境变量覆盖，测试用）；`dozer-hook install codex`/`dozer-hook uninstall codex` 能装/卸 `~/.codex/hooks.json`。

- [ ] **Step 1: 写失败的测试**

`install.rs` 现有测试模块追加：

```rust
    #[test]
    fn install_writes_agent_specific_command_for_codex() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        assert_eq!(run_at(&path, "codex", true), 0);
        let root = read(&path);
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains(" codex "), "{cmd}");
    }

    #[test]
    fn settings_path_for_codex_points_at_codex_hooks_json() {
        unsafe { std::env::set_var("DOZER_CODEX_SETTINGS", "/tmp/probe-codex.json") };
        assert_eq!(
            settings_path_for("codex"),
            std::path::PathBuf::from("/tmp/probe-codex.json")
        );
        unsafe { std::env::remove_var("DOZER_CODEX_SETTINGS") };
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook codex -- --nocapture`
Expected: FAIL，`settings_path_for("codex")` 落到 Claude 分支（`_` 兜底），路径不匹配。

- [ ] **Step 3: 实现**

`install.rs` 第 21-38 行的 `settings_path_for`：

```rust
/// agent 名 → 该 agent 的 hook 配置文件路径。CodeBuddy/Codex 走各自的
/// 全局配置文件（与 Claude 同构的 JSON 补丁机制，Codex 的结构已通过官方
/// 文档核实、Task 4 spike 现场验证），环境变量覆盖用于测试，跟既有 Claude
/// 路径同一套手法。
pub fn settings_path_for(agent: &str) -> PathBuf {
    match agent {
        "codebuddy" => {
            if let Ok(p) = std::env::var("DOZER_CODEBUDDY_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".codebuddy").join("settings.json")
        }
        "codex" => {
            if let Ok(p) = std::env::var("DOZER_CODEX_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".codex").join("hooks.json")
        }
        _ => {
            if let Ok(p) = std::env::var("DOZER_CLAUDE_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".claude").join("settings.json")
        }
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS，全量 `dozer-hook` 测试绿。

- [ ] **Step 5: 全 workspace 编译 + 测试确认无回归**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 全部 crate 编译通过、测试全绿。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-hook/src/install.rs
git commit -m "feat(dozer-hook): Codex hook installer (hooks.json patch)"
```

---

## Task 7: Spike——验证 Qoder hook 注册与 transcript 落盘

人工验证任务。**Qoder CLI 需要先安装**（官方安装脚本，来自第三方域名，执行前请自行确认信任该脚本来源）：

```bash
curl -fsSL https://qoder.com/install | bash
```

**Files:**
- Create: `crates/dozer-hook/fixtures/qoder-transcript-sample.jsonl`（若拿到可脱敏样本）
- Create: `docs/superpowers/specs/2026-08-07-qoder-spike-findings.md`

- [ ] **Step 1: 确认 Qoder CLI 可用**

Run: `qoder --version`
Expected: 打印版本号。若未安装，先跑上面的官方安装脚本。

- [ ] **Step 2: 备份现有配置**

```bash
mkdir -p ~/.qoder
cp -n ~/.qoder/settings.json ~/.qoder/settings.json.dozer-spike-backup 2>/dev/null || true
```

- [ ] **Step 3: 写探针 settings.json，跑一次会话**

写 `~/.qoder/settings.json`（若原文件非空，先手工把下面的 `hooks` 段合并进去，不要整体覆盖）：

```json
{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===SessionStart===\\n\" >> /tmp/qoder-hook-probe.log; cat >> /tmp/qoder-hook-probe.log'" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===UserPromptSubmit===\\n\" >> /tmp/qoder-hook-probe.log; cat >> /tmp/qoder-hook-probe.log'" }] }],
    "PreToolUse": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===PreToolUse===\\n\" >> /tmp/qoder-hook-probe.log; cat >> /tmp/qoder-hook-probe.log'" }] }],
    "PostToolUse": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===PostToolUse===\\n\" >> /tmp/qoder-hook-probe.log; cat >> /tmp/qoder-hook-probe.log'" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===Stop===\\n\" >> /tmp/qoder-hook-probe.log; cat >> /tmp/qoder-hook-probe.log'" }] }],
    "SessionEnd": [{ "hooks": [{ "type": "command", "command": "sh -c 'printf \"\\n===SessionEnd===\\n\" >> /tmp/qoder-hook-probe.log; cat >> /tmp/qoder-hook-probe.log'" }] }]
  }
}
```

跑一个真实/半真实会话触发这些事件（Qoder 是否有类似 CodeBuddy `-p --dangerously-skip-permissions` 的非交互一次性模式，以 `qoder --help` 实测为准；没有的话手动跑一轮交互会话、正常问答后退出）。

- [ ] **Step 4: 检查探测日志里的字段**

```bash
cat /tmp/qoder-hook-probe.log
```

确认每个事件段是否含 `session_id`/`transcript_path`/`cwd`/`hook_event_name`/`tool_name`/`tool_input` 字段。记下实际字段名——如果跟假设不一致，Task 8 的翻译表要跟着改。

- [ ] **Step 5: 抓一份真实 transcript 样本**

同 Task 4 Step 5 的做法，从 `transcript_path` 指向的文件脱敏后存到 `crates/dozer-hook/fixtures/qoder-transcript-sample.jsonl`。

- [ ] **Step 6: 还原配置**

```bash
if [ -f ~/.qoder/settings.json.dozer-spike-backup ]; then
  mv ~/.qoder/settings.json.dozer-spike-backup ~/.qoder/settings.json
else
  rm -f ~/.qoder/settings.json
fi
rm -f /tmp/qoder-hook-probe.log
```

- [ ] **Step 7: 写决策记录**

创建 `docs/superpowers/specs/2026-08-07-qoder-spike-findings.md`，格式同 Task 4 Step 7。

- [ ] **Step 8: 提交**

```bash
git add docs/superpowers/specs/2026-08-07-qoder-spike-findings.md
git add crates/dozer-hook/fixtures/qoder-transcript-sample.jsonl 2>/dev/null || true
git commit -m "docs: record Qoder hook registration spike findings"
```

---

## Task 8: Qoder 事件名翻译表 + `forward()` 接入

**Files:**
- Create: `crates/dozer-hook/src/qoder.rs`
- Modify: `crates/dozer-hook/src/main.rs`（顶部 `mod` 声明、`parse_agent`、`resolve_event`）

**Interfaces:**
- Consumes: `AgentKind::Qoder`（Task 1）；Task 7 spike 结论。
- Produces: `qoder::translate_event(raw: &str) -> Option<String>`；`parse_agent("qoder") == AgentKind::Qoder`；`resolve_event(AgentKind::Qoder, ..)` 套用该翻译表。

- [ ] **Step 1: 写失败的测试**

创建 `crates/dozer-hook/src/qoder.rs`：

```rust
//! Qoder 原生 hook 事件名 → dozerd 规范事件名翻译（spec §6）。规范词汇表
//! 就是 dozerd::agent_state_for 认识的 7 个 Claude 事件名。Qoder 的事件
//! 名词汇（20 个，跨会话/工具/agent flow/压缩/通知/文件六大类）经官方
//! 文档核实（`docs.qoder.com/en/cli/hooks`），字段名与 Claude 逐字对齐。
//! Task 7 spike 若发现实测偏差，回来改这张表即可，不影响其余逻辑。

/// `None` = 这个事件不转发给 dozerd（压缩/子 agent/文件变更/worktree/MCP
/// elicitation 等不影响顶层四态机的事件）。未知事件原样透传。
pub fn translate_event(raw: &str) -> Option<String> {
    match raw {
        "SessionStart" => Some("SessionStart"),
        "SessionEnd" => Some("SessionEnd"),
        "UserPromptSubmit" => Some("UserPromptSubmit"),
        "PreToolUse" => Some("PreToolUse"),
        "PostToolUse" | "PostToolUseFailure" => Some("PostToolUse"),
        "Stop" | "StopFailure" => Some("Stop"),
        "Notification" | "PermissionRequest" | "PermissionDenied" => Some("Notification"),
        "SubagentStart" | "SubagentStop" | "PreCompact" | "PostCompact" | "InstructionsLoaded"
        | "ConfigChange" | "CwdChanged" | "FileChanged" | "WorktreeCreate" | "WorktreeRemove"
        | "Elicitation" | "ElicitationResult" => return None,
        other => other,
    }
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_events_map_to_same_name() {
        assert_eq!(translate_event("SessionStart"), Some("SessionStart".to_string()));
        assert_eq!(translate_event("SessionEnd"), Some("SessionEnd".to_string()));
        assert_eq!(translate_event("UserPromptSubmit"), Some("UserPromptSubmit".to_string()));
        assert_eq!(translate_event("PreToolUse"), Some("PreToolUse".to_string()));
        assert_eq!(translate_event("PostToolUse"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("Stop"), Some("Stop".to_string()));
        assert_eq!(translate_event("Notification"), Some("Notification".to_string()));
    }

    #[test]
    fn failure_variants_merge_into_success_variant() {
        assert_eq!(translate_event("PostToolUseFailure"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("StopFailure"), Some("Stop".to_string()));
    }

    #[test]
    fn permission_events_merge_into_notification() {
        assert_eq!(translate_event("PermissionRequest"), Some("Notification".to_string()));
        assert_eq!(translate_event("PermissionDenied"), Some("Notification".to_string()));
    }

    #[test]
    fn non_state_machine_events_are_dropped() {
        for ev in [
            "SubagentStart",
            "SubagentStop",
            "PreCompact",
            "PostCompact",
            "InstructionsLoaded",
            "ConfigChange",
            "CwdChanged",
            "FileChanged",
            "WorktreeCreate",
            "WorktreeRemove",
            "Elicitation",
            "ElicitationResult",
        ] {
            assert_eq!(translate_event(ev), None, "{ev}");
        }
    }

    #[test]
    fn unknown_event_passes_through_unmapped() {
        assert_eq!(translate_event("SomeFutureEvent"), Some("SomeFutureEvent".to_string()));
    }
}
```

`main.rs` 顶部 `mod` 声明追加：

```rust
mod qoder;
```

`parse_agent` 追加一行：

```rust
        "qoder" => AgentKind::Qoder,
```

`resolve_event` 追加一支：

```rust
        AgentKind::Qoder => qoder::translate_event(&raw),
```

`main.rs` 测试模块追加：

```rust
    #[test]
    fn parse_agent_recognizes_qoder() {
        assert_eq!(parse_agent("qoder"), AgentKind::Qoder);
    }

    #[test]
    fn resolve_event_translates_qoder_failure_variants() {
        assert_eq!(
            resolve_event(AgentKind::Qoder, Some("PostToolUseFailure")),
            Some("PostToolUse".to_string())
        );
    }

    #[test]
    fn resolve_event_drops_qoder_file_changed() {
        assert_eq!(resolve_event(AgentKind::Qoder, Some("FileChanged")), None);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook qoder -- --nocapture`
Expected: FAIL，`crates/dozer-hook/src/qoder.rs` 不存在，编译错误。

- [ ] **Step 3: 实现**

已在 Step 1 给出全部实现代码，直接落地。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-hook/src/qoder.rs crates/dozer-hook/src/main.rs
git commit -m "feat(dozer-hook): Qoder native event name translation table"
```

---

## Task 9: Qoder hook 安装器

**Files:**
- Modify: `crates/dozer-hook/src/install.rs`（`settings_path_for` 追加 `"qoder"` 分支）
- Modify: `crates/dozer-hook/src/install.rs`（既有测试模块追加 Qoder 用例）

**Interfaces:**
- Consumes: `install::run_at`（不变）。
- Produces: `settings_path_for("qoder") -> ~/.qoder/settings.json`（`DOZER_QODER_SETTINGS` 覆盖）。

- [ ] **Step 1: 写失败的测试**

```rust
    #[test]
    fn install_writes_agent_specific_command_for_qoder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(run_at(&path, "qoder", true), 0);
        let root = read(&path);
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains(" qoder "), "{cmd}");
    }

    #[test]
    fn settings_path_for_qoder_points_at_qoder_dir() {
        unsafe { std::env::set_var("DOZER_QODER_SETTINGS", "/tmp/probe-qoder.json") };
        assert_eq!(
            settings_path_for("qoder"),
            std::path::PathBuf::from("/tmp/probe-qoder.json")
        );
        unsafe { std::env::remove_var("DOZER_QODER_SETTINGS") };
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-hook qoder -- --nocapture`
Expected: FAIL，`settings_path_for("qoder")` 落到 Claude 分支兜底，路径不匹配。

- [ ] **Step 3: 实现**

在 `settings_path_for` 的 `"codex" => { ... }` 分支之后追加：

```rust
        "qoder" => {
            if let Ok(p) = std::env::var("DOZER_QODER_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".qoder").join("settings.json")
        }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-hook -- --nocapture`
Expected: PASS，全量 `dozer-hook` 测试绿。

- [ ] **Step 5: 全 workspace 编译 + 测试确认无回归**

Run: `cargo build --workspace && cargo test --workspace`
Expected: 全部 crate 编译通过、测试全绿。

- [ ] **Step 6: 提交**

```bash
git add crates/dozer-hook/src/install.rs
git commit -m "feat(dozer-hook): Qoder hook installer (settings.json patch)"
```

---

## Task 10: Spike——验证 Kilo 插件可行性（go/no-go 决策）

人工验证任务，**本任务的产出直接决定 Kilo 是否值得继续投入**。背景：Kilo 官方插件文档（`kilo.ai/docs/automate/extending/plugins`）描述了一个 `event` hook，可以订阅 `session.*`/`message.*`/`tool.execute.*` 等内部总线事件，形态与 OpenCode 插件几乎一致（`Kilo-Org/kilocode` 仓库内部确实有一个字面上叫 `packages/opencode` 的包，印证 Kilo 是 OpenCode 的下游分支）；但 Kilo 官方一个 issue（`Kilo-Org/kilocode#5827`，"Expose session lifecycle hooks for third-party tool integration"）被 "Closed as not planned"，暗示"暴露生命周期钩子给第三方"这件事本身可能不受官方支持——两者是否矛盾、插件机制在真实 CLI 里是否真的工作，必须实测，不能只信文档。

**Files:**
- Create: `docs/superpowers/specs/2026-08-07-kilo-spike-findings.md`

- [ ] **Step 1: 安装 Kilo CLI**

```bash
npm install -g @kilocode/cli
kilo --version
```

Expected: 打印版本号（官方包 `@kilocode/cli`，`bin` 字段暴露 `kilo`/`kilocode` 两个命令别名，已通过 `npm view @kilocode/cli bin` 核实）。

- [ ] **Step 2: 写一个最小探针插件**

创建项目目录 `/tmp/kilo-spike-project`，在其中创建 `.kilo/plugin/dozer-probe.ts`：

```typescript
import type { Plugin } from "@kilocode/cli" // 若包名/类型导出路径与预期不同，记录实际路径

const probe: Plugin = async () => ({
  event: async ({ event }: { event: { type: string; properties?: unknown } }) => {
    const fs = await import("fs")
    fs.appendFileSync(
      "/tmp/kilo-hook-probe.log",
      `\n===${event.type}===\n${JSON.stringify(event.properties ?? {}, null, 2)}\n`
    )
  },
})

export default probe
```

（如果 `import type { Plugin } from "@kilocode/cli"` 编译/加载失败，尝试实际包导出的类型路径——记录真实可用的 import 路径，这本身就是本次 spike 要验证的一部分。）

- [ ] **Step 3: 确认插件被加载、事件确实到达**

```bash
cd /tmp/kilo-spike-project && rm -f /tmp/kilo-hook-probe.log
kilo "reply with exactly one word: hello"
cat /tmp/kilo-hook-probe.log
```

Expected 三种可能之一：
- **完全命中**：日志里能看到 `session.created`/`message.updated`/`tool.execute.before`/`session.idle` 等事件，且 `event.properties` 里能找到可用的 session 标识字段（如 `sessionID`/`info.id`）——go：Kilo 插件可行，翻译逻辑照 spec §7 写一个独立后续计划。
- **部分命中**（比如插件被加载但只有部分事件类型触发，或事件到达但字段形状与预期不同）：记录实际能拿到什么、拿不到什么，可能仍然 go，但翻译表要按实测重新设计。
- **完全不命中**（插件根本不被加载 / `.kilo/plugin/` 目录不存在这套机制 / API 与文档不符）：no-go——记录清楚失败点，Kilo adapter 按 spec §7/本计划 Global Constraints 的降级路径，维持"仅有诚实降级分支、无插件实现"的状态，不再投入。

- [ ] **Step 4: 确认 `DOZER_SESSION_ID` 能否被插件读到**

```bash
DOZER_SESSION_ID=probe-123 kilo "reply with exactly one word: hello"
```

检查探针插件里 `process.env.DOZER_SESSION_ID` 是否等于 `"probe-123"`（可以在 Step 2 的探针里加一行 `process.env.DOZER_SESSION_ID` 写入日志）。这决定 Kilo 插件能否像 OpenCode 插件一样单纯读 `process.env` 做会话关联。

- [ ] **Step 5: 确认 `.kilo/plugin/` 是否平铺加载**

在同一目录下再放一个纯工具文件 `.kilo/plugin/dozer-lib-probe.ts`（不导出 `Plugin` 类型的具名函数，就导出一个普通函数），重新跑一次 `kilo`，观察 Kilo 是否尝试把这个文件也当插件加载并报错（若报错，说明跟 OpenCode 一样需要把辅助逻辑塞进子目录）。

- [ ] **Step 6: 写决策记录**

创建 `docs/superpowers/specs/2026-08-07-kilo-spike-findings.md`，内容至少包含：go/no-go 结论、真实可用的插件 import 路径与类型、真实观察到的事件类型与 `event.properties` 字段形状、`DOZER_SESSION_ID` 传递是否可行、`.kilo/plugin/` 是否平铺加载。若结论是 go，本文档同时是下一个"Kilo 插件实现"计划的输入；若 no-go，本文档就是"为什么不做"的最终记录。

- [ ] **Step 7: 清理**

```bash
rm -rf /tmp/kilo-spike-project /tmp/kilo-hook-probe.log
npm uninstall -g @kilocode/cli
```

（是否保留 Kilo CLI 全局安装由执行者自行判断；本计划不假定后续一定需要它。）

- [ ] **Step 8: 提交**

```bash
git add docs/superpowers/specs/2026-08-07-kilo-spike-findings.md
git commit -m "docs: record Kilo plugin feasibility spike findings (go/no-go)"
```

---

## 完成检查

- [x] `AgentKind` 有 7 个变体（含新增 `Codex`/`Qoder`/`Kilo`），`label()` 全部覆盖，老协议帧回落 `Unknown` 的既有行为不变。
- [x] 启动器菜单 8 项，Codex/Qoder/Kilo 键入正确的 CLI 命令名（`codex`/`qoder`/`kilo`），各自有独立强调色，图标回落 `Bot`（诚实降级，非占位符）。
- [x] `dozer-hook install codex` / `dozer-hook install qoder` 能正确装 `~/.codex/hooks.json` / `~/.qoder/settings.json`；`Notification`/`PostToolUseFailure` 等归并、`Subagent*`/压缩类事件正确丢弃、未知事件透传不 panic。
- [x] `cargo build --workspace && cargo test --workspace` 全绿。
- [x] 三份 spike 决策记录已写清楚（`2026-08-07-{codex,qoder,kilo}-spike-findings.md`），Kilo 的记录明确给出 go/no-go 结论。
- [ ] **明确排除在本计划外，留给独立后续计划**：
  - Codex/Qoder 的真实 transcript 解析（`transcript.rs`/`usage.rs` 里这两家的分支目前诚实返回空/默认值）与历史对话目录扫描（`conversation.rs::list_all_conversations` 不含这两家）——依赖各自 spike 产出的 fixture 与目录结构核实结果。
  - Kilo 插件代码（`dozer.ts`/`dozer-translate.ts`/`kilo_install.rs`）——依赖 Task 10 的 go/no-go 结论；如果 go，需要一份新的 brainstorming/plan 周期（涉及 TypeScript 代码、不是本计划"纯 Rust 机械扩展"的范畴）。
  - `usage.rs` 用量面板的 `group_usage_by_agent`/`daily_totals_by_agent`/`agent_token_share` 尚未扩展到真正统计 Codex/Qoder/Kilo 的用量（目前这三家在这些函数里要么不出现、要么被当"无专属分桶"处理）——在没有真实数据源（即上一条的 transcript 解析）之前扩展这些函数没有意义，属于同一个后续计划的一部分。
