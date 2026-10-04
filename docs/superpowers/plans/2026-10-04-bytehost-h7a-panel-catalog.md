# bytehost H7a:面板注册清单(`PanelCatalog`)与产品组合根——默认栏位/顺序/标题/图标/布局校验改由清单驱动 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地设计文档 H7 的第一半(`docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md` §3.4、§4,用户裁决 O5):host 不再在代码里写死"有哪 12 个面板、默认放哪、什么顺序、什么图标、什么标题",而是认一份由**产品组合根**在启动时注册的清单。Dozer 的清单写在新文件 `product.rs`;换一个产品(Digger)就是另写一份。**不改任何用户可见行为,落盘格式不变。**

**Architecture:** 一个代码任务 + 一个文档任务。Task 1 先写**特征测试**(4 个,针对旧代码,必须先过——它们是"逐字不变"的依据),再写新模块 `panel_registry.rs` 的测试(6 个,先看编译失败),然后跑实现脚本:`PanelCatalog`(描述符 + 默认布局 + 旧布局迁移钩子,构造时校验)、全局持有者 `install`/`catalog`、`product::dozer_catalog()`,并把 `PanelKind::default_side`、`RailLayout::default`、`panel_meta`、`sanitize_rail_layout` 改走清单;6 个变异检验证明测试会失败。Task 2 回填文档。

**Tech Stack:** Rust(`dozer-app`);Python 3 标准库(一次性脚本)。不碰 `Cargo.toml`/`Cargo.lock`。

**Spec:** 设计文档 §1(O5 已定)、§3.4、§4 H7、§6.3(兼容测试);H0 `01-panelkind.md` §5–§6(B1/B2)。

## 对设计文档的细化(请评审)

1. **H7 拆成 H7a(本计划)与 H7b(`PanelKind`→`PanelId`),H7b 随 H8 评估。** 设计文档把"`PanelId` 类型 + 描述符/注册表 + 组合根默认栏位"放在同一步。读代码后:`PanelKind` 在 30 多个文件里出现 1100 多次,其中 `PanelKind::Files` 之类点名变体的 match/比较占大头;把它换成字符串 newtype 是纯机械的类型替换,但**在 H8(每面板的 `Workspace` 状态字段、`PanelDims` 字段、`Message` 的 20 个包装变体动态化)之前没有任何收益**——换了类型,这些地方照样要点名 12 个面板。而清单(本刀)是有独立收益的:默认栏位/顺序/图标/标题/校验不再写死在 host 里,且已经能被一个更小的清单(Digger 的 3 个面板)驱动(测试里用小清单证明)。所以先做有收益的一半。
2. **`PanelDescriptor` 里没有 `hooks`。** 设计文档 §3.4 写"`PanelDescriptor { id, title, icon, needs_preview_column, hooks }`"。H6 之后看清楚了:各面板的 `on_activate` 签名不同(有的要 `&mut` 面板状态,有的要 `ActivationCtx<自己的 Message>`,消息类型各异),放进清单需要统一成"吃 `&mut App` 的闭包",这等于把 host 的 `match` 搬进每个面板的适配闭包,并没有解耦——属于 H8(状态/消息信封动态化)的范围。`fire_panel_switch_in` 因此仍是 host 的 `match`(H6 补注已写)。`needs_preview_column` 同理推迟:它对应的 `webview_geometry` 里的几何公式是按面板各写各的,不是一个布尔能表达的。
3. **清单是进程级全局(`OnceLock`),不是注入 `App`。** `RailLayout::default()`、`PanelKind::default_side()` 这类函数没有 `App`/`ShellLayout` 可取,在几十个调用点(含大量测试)里被直接调用;改成显式传参要动上百处签名。全局的代价是测试共享同一份 Dozer 清单(清单逻辑本身用**局部构造的小清单**测,不经全局),`install` 只允许一次。生产里 `catalog()` 在未注册时 panic 并说明原因,`main()` 在任何布局读取之前注册。

## Global Constraints

- **行为保持。** 默认布局两栏的面板与顺序、12 个图标与标题、`sanitize_rail_layout` 的合法/非法判定(含旧 11 面板迁移)、落盘 JSON 里的枚举名,逐字不变。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-h7/...`),分支 `bytehost-h7`,从当前 `main` 开。主 checkout 常有并发会话与未提交改动(此刻 `crates/dozerd/src/transcripts/mod.rs` 就是别人的),不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **`Cargo.lock`:** 本地 `.cargo/config.toml` 的 `[patch]` 会让 `Cargo.lock` 里 `byteui`/`bytegit` 的 `source = …` 行消失。**每次提交前 `git checkout -- Cargo.lock`**;`.cargo/config.toml` 不提交;本计划不改依赖。
- **变异检验前先 `git add` 暂存当前版本**(新文件也要 `git add`),变异后用 `git checkout -- crates` 还原到暂存版本。
- **每次改 Rust 后跑 `cargo fmt -p dozer-app`,再用 `git status --short` 确认只有本任务触碰的文件变了。**
- **编译与测试一律带 `RUSTFLAGS="-D unconditional_recursion"`**(沿用 H1 的防线)。
- **heredoc 一律用带引号的分隔符(`<<'EOF'`)写脚本,绝不用不带引号的**。需要传值给脚本时用环境变量(`H7=… python3 script.py`)。
- **clippy 诊断必须与 `main` 逐文件一致**;`panel_registry.rs` 里每个 `pub(crate)` 方法在非测试代码里都要有调用者(草稿里 `len()` 一度只被测试用到,触发 dead_code,已改成被 `accepts` 使用)。
- 提交信息用 `refactor:`/`test:`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 稳定失败;`assets::tests::serves_vendored_asset_with_mime` 偶发失败(重跑即过);`extensions::git_log::tests::build_marks_head_branch_and_labels`、`build_populates_time_and_is_merge` 在 linked worktree 里可能失败。
- 草稿基线(`main` = H6 之后,linked worktree):`1766 passed; 1 failed`。草稿里 Task 1 之后 `1776 passed; 1 failed`(+10 个新测试:4 个特征测试 + 6 个清单测试)。门禁(`check_panel_boundary.py`)输出 `ok (122 refs …)`,前后不变。

## Review Focus

1. **`catalog()` 在生产里未注册就 panic:** `main()` 里 `panel_registry::install(product::dozer_catalog())` 必须在**任何**读布局、建 `ShellLayout`、调 `default_side()` 之前。草稿里把它放在日志初始化之后、字体/主题初始化之前;审阅时确认 `main()` 之前没有别的入口(例如 `#[ctor]`、静态初始化)会触发 `RailLayout::default()`。测试里 `catalog()` 自动装 Dozer 清单,所以这条**没有测试能证明**,只能读 diff。
2. **`panic!`/`expect` 的边界:** `default_side()`/`panel_meta()` 对"没注册的面板"panic。Dozer 的清单覆盖 `PanelKind` 全部 12 个变体(测试 `dozer_catalog_registers_every_panel_kind_exactly_once` 里的穷举 `match` 保证新增变体时必须同步),所以不会触发;但**以后换成比枚举更小的清单**(Digger)时,host 里凡是对"枚举的所有变体"遍历再取元数据的代码都会 panic——这是 H7b/H8 要面对的,本刀不提前处理。
3. **`sanitize_rail_layout` 判定等价性:** 旧写法是"任一栏空 → 默认;`all.len() != 12 || 两栏长度和 != 12` → 默认"。新写法 `accepts` = 两栏非空 ∧ 无重复 ∧ 总数 == 清单长度 ∧ 全部已注册。对封闭的 12 变体枚举与 12 面板的 Dozer 清单,两者对任何输入判定相同(特征测试 + 既有的 `sanitize_*` 测试一起固定)。
4. **`migrate_legacy_rail` 没改,仍在 `rail.rs`:** 它是 Dozer 自己的历史(旧版没有群聊面板,"11"是历史事实,不是当前面板数),所以作为组合根给清单的**钩子**传入,而不是泛化。它的既有测试原地不动。
5. **全局可变性:** `OnceLock` 只允许 `set` 一次(`install` 第二次返回错误、不覆盖;测试 `install_is_one_shot` 固定)。测试进程里全局清单是 Dozer 的,依赖"自己的清单"的逻辑必须用局部构造(本计划的清单测试都是)。
6. **没做的(本刀明确不碰):** `PanelKind`→`PanelId`(H7b)、清单里放钩子、`needs_preview_column`、`webview_geometry` 的 12 臂 match、`app/layout.rs` 里按面板展开的布局代码、`PanelDims`/`Workspace` 的每面板字段。它们在设计文档 §3.4 补注里有理由。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/dozer-app/src/panel_registry.rs`(新) | `PanelDescriptor`、`PanelCatalog`(校验/查询/`accepts`)、全局 `install`/`catalog`;6 个测试 | 1 |
| `crates/dozer-app/src/product.rs`(新) | Dozer 组合根 `dozer_catalog()`(12 个描述符、默认两栏、旧布局迁移钩子) | 1 |
| `crates/dozer-app/src/main.rs` | 挂两个模块;启动最前 `install` | 1 |
| `crates/dozer-app/src/app/state.rs` | `PanelKind::default_side` 改查清单 | 1 |
| `crates/dozer-app/src/chrome/rail.rs` | `Default for RailLayout`、`panel_meta`、`sanitize_rail_layout` 改走清单;4 个特征测试 | 1 |
| `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`、`docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md` | 回填 | 2 |

一次性脚本(**不提交**)放 `$SCRATCH`(仓库外)。

---

### Task 1: `PanelCatalog` 与 Dozer 组合根

**Files:** 见上表(`crates/` 下 6 个文件,其中 2 个新增)。

**Interfaces:**
- Produces(Task 2 与 H7b/H8 依赖):`crate::panel_registry::{PanelDescriptor { id: PanelKind, title: &'static str, icon: IconKind }, PanelCatalog, install(PanelCatalog) -> Result<(), &'static str>, catalog() -> &'static PanelCatalog}`;`PanelCatalog::{new(Vec<PanelDescriptor>, Vec<PanelKind> /*左*/, Vec<PanelKind> /*右*/, fn(RailLayout) -> RailLayout) -> Result<Self, String>, len() -> usize, descriptor(PanelKind) -> Option<&PanelDescriptor>, default_rail() -> RailLayout, default_side(PanelKind) -> Option<Side>, migrate_legacy(RailLayout) -> RailLayout, accepts(&RailLayout) -> bool}`(均 `pub(crate)`);`crate::product::dozer_catalog() -> PanelCatalog`。

- [ ] **Step 1: 建 worktree、分支、脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-h7 -b bytehost-h7 main
mkdir -p ../dozer-bytehost-h7/.cargo && cp .cargo/config.toml ../dozer-bytehost-h7/.cargo/config.toml
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7 && git log --oneline | head -1 && git status --short
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-h7-scratch && mkdir -p $SCRATCH/h7
```

Expected: 干净、分支 `bytehost-h7`。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-h7/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 记录基线**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7 && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^test result|FAILED" | tee $SCRATCH/baseline.txt; git checkout -- Cargo.lock`
Expected: 失败只有"已知基线"里列的;记下通过数 N(草稿:1766)。再记 clippy 基线:`RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-before.txt; git checkout -- Cargo.lock`(草稿:11 个文件有既有诊断)。

- [ ] **Step 3: 写特征测试(针对旧代码,必须**先过**)**

把下面写成 `$SCRATCH/h7/tests_rail_golden.rs`(保留开头空行):

```rust

    // ---- H7:面板清单的特征测试(在迁移前后都必须通过) ----

    /// 默认栏位与默认顺序的**字面值**:`RailLayout::default()` 与 `default_side()` 迁到
    /// 注册清单之后,这张表是"逐字不变"的唯一依据。
    #[test]
    fn golden_default_layout_order_and_sides() {
        use PanelKind::*;
        let rail = RailLayout::default();
        assert_eq!(
            rail.left,
            vec![Project, Todo, Files, GitLog, Database, Ssh, Web]
        );
        assert_eq!(
            rail.right,
            vec![Agent, GroupChat, Conversations, Usage, CodeHealth]
        );
        for kind in [Files, GitLog, Todo, Project, Database, Ssh, Web] {
            assert_eq!(kind.default_side(), Side::Left, "{kind:?}");
        }
        for kind in [Agent, GroupChat, Conversations, Usage, CodeHealth] {
            assert_eq!(kind.default_side(), Side::Right, "{kind:?}");
        }
    }

    /// 12 个面板的图标与 tooltip 文案的字面值。
    #[test]
    fn golden_panel_icons_and_titles() {
        use icons::IconKind as I;
        use PanelKind::*;
        let expected = [
            (Files, I::FolderTree, "文件"),
            (GitLog, I::GitGraph, "Git Log"),
            (Todo, I::ListTodo, "待办"),
            (Project, I::Briefcase, "项目"),
            (Database, I::Database, "数据库"),
            (Ssh, I::Server, "SSH 主机"),
            (Web, I::Globe, "浏览器"),
            (Agent, I::Brain, "代理"),
            (GroupChat, I::SquareSparkles, "群聊"),
            (Conversations, I::BotMessageSquare, "对话"),
            (Usage, I::BarChart3, "用量"),
            (CodeHealth, I::SquareActivity, "代码健康度"),
        ];
        for (kind, icon, title) in expected {
            let (got_icon, got_title) = panel_meta(kind);
            assert_eq!(got_icon, icon, "{kind:?} 图标");
            assert_eq!(got_title, title, "{kind:?} 文案");
        }
    }

    /// 落盘兼容:旧版 `layout.json` 里的 `rail_layout` 用**枚举名字符串**,12 个名字逐一
    /// 能反序列化、再序列化出同一个字符串;12 个面板的合法布局经消毒后原样保留。
    #[test]
    fn golden_rail_layout_json_uses_variant_names_and_survives_sanitize() {
        let json = r#"{"left":["Project","Todo","Files","GitLog","Database","Ssh","Web"],
                       "right":["Agent","GroupChat","Conversations","Usage","CodeHealth"]}"#;
        let rail: RailLayout = serde_json::from_str(json).unwrap();
        assert_eq!(rail, RailLayout::default());
        assert_eq!(sanitize_rail_layout(rail.clone()), rail);
        let out = serde_json::to_value(&rail).unwrap();
        assert_eq!(out["left"][0], "Project");
        assert_eq!(out["right"][4], "CodeHealth");
    }

    /// 旧版(11 个面板、没有群聊)的 `rail_layout` 经消毒后迁移成 12 个,群聊紧跟在 Agent 后。
    #[test]
    fn golden_legacy_eleven_panel_json_is_migrated() {
        let json = r#"{"left":["Project","Todo","Files","GitLog","Database","Ssh","Web"],
                       "right":["Agent","Conversations","Usage","CodeHealth"]}"#;
        let rail: RailLayout = serde_json::from_str(json).unwrap();
        assert_eq!(sanitize_rail_layout(rail), RailLayout::default());
    }
```

再写追加脚本 `$SCRATCH/h7_golden.py`(带引号 heredoc),运行并跑这 4 个测试:

```python
#!/usr/bin/env python3
"""H7 一次性脚本(步骤 3):把特征测试追加到 `chrome/rail.rs` 的 `mod tests` 末尾。`SP` 是 tests_*.rs 所在目录。**不提交。**"""
import os

p = "crates/dozer-app/src/chrome/rail.rs"
s = open(p).read()
assert s.rstrip("\n").endswith("\n}"), p
body = open(os.path.join(os.environ["SP"], "tests_rail_golden.rs")).read().rstrip("\n")
open(p, "w").write(s.rstrip("\n")[:-1].rstrip("\n") + "\n" + body + "\n}\n")
print("ok")
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/h7 python3 $SCRATCH/h7_golden.py
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app golden_ 2>&1 | grep -E "^error|^test |test result" -A3; git checkout -- Cargo.lock
```
Expected: 脚本输出 `ok`;4 个 `golden_*` 测试(外加已有的 `apply_column_drag_matches_the_golden_table`)**全部通过**——特征测试在旧实现上就该是绿的,这证明它们描述的是现有行为;若有失败,先查是测试写错还是 `main` 上行为已变,**不要**改实现去迎合。

- [ ] **Step 4: 写清单的测试(新文件,先看编译失败)**

把下面写成 `$SCRATCH/h7/tests_panel_registry.rs`:

```rust
//! (H7)面板注册清单的测试:`PanelCatalog` 的构造校验与查询(用小清单,不依赖 Dozer 的 12 个面板),
//! 以及 Dozer 组合根清单的自检。

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{PanelKind, Side};
    use crate::chrome::rail::RailLayout;
    use byteui::interaction::icons::IconKind;

    fn desc(id: PanelKind) -> PanelDescriptor {
        PanelDescriptor {
            id,
            title: "t",
            icon: IconKind::Globe,
        }
    }

    fn keep(rail: RailLayout) -> RailLayout {
        rail
    }

    /// Digger 式的 3 面板小清单:Files、Todo 在左,Agent 在右。
    fn small() -> PanelCatalog {
        PanelCatalog::new(
            vec![
                desc(PanelKind::Files),
                desc(PanelKind::Todo),
                desc(PanelKind::Agent),
            ],
            vec![PanelKind::Todo, PanelKind::Files],
            vec![PanelKind::Agent],
            keep,
        )
        .unwrap()
    }

    fn rail(left: &[PanelKind], right: &[PanelKind]) -> RailLayout {
        RailLayout {
            left: left.to_vec(),
            right: right.to_vec(),
        }
    }

    #[test]
    fn default_rail_and_sides_follow_the_composition_roots_layout_not_the_enum() {
        let c = small();
        assert_eq!(c.len(), 3);
        assert_eq!(
            c.default_rail(),
            rail(&[PanelKind::Todo, PanelKind::Files], &[PanelKind::Agent])
        );
        assert_eq!(c.default_side(PanelKind::Files), Some(Side::Left));
        assert_eq!(c.default_side(PanelKind::Agent), Some(Side::Right));
        assert_eq!(c.default_side(PanelKind::Web), None, "没注册的面板没有默认栏");
        assert!(c.descriptor(PanelKind::Todo).is_some());
        assert!(c.descriptor(PanelKind::Web).is_none());
    }

    #[test]
    fn accepts_exactly_the_registered_panels_in_any_arrangement() {
        use PanelKind::*;
        let c = small();
        assert!(c.accepts(&rail(&[Todo, Files], &[Agent])));
        assert!(c.accepts(&rail(&[Agent], &[Files, Todo])), "换栏换序都合法");
        assert!(!c.accepts(&rail(&[Todo, Files], &[])), "任一栏为空不合法");
        assert!(!c.accepts(&rail(&[], &[Todo, Files, Agent])));
        assert!(!c.accepts(&rail(&[Todo], &[Agent])), "缺一个面板");
        assert!(!c.accepts(&rail(&[Todo, Todo], &[Agent])), "重复");
        assert!(
            !c.accepts(&rail(&[Todo, Files, Web], &[Agent])),
            "多出清单之外的面板"
        );
    }

    #[test]
    fn new_rejects_inconsistent_catalogs() {
        use PanelKind::*;
        let go = |descs: Vec<PanelKind>, l: Vec<PanelKind>, r: Vec<PanelKind>| {
            PanelCatalog::new(descs.into_iter().map(desc).collect(), l, r, keep).err()
        };
        assert!(go(vec![Files, Agent], vec![Files], vec![Agent]).is_none());
        assert!(go(vec![], vec![], vec![]).is_some(), "空清单");
        assert!(go(vec![Files, Files, Agent], vec![Files], vec![Agent]).is_some(), "描述重复");
        assert!(go(vec![Files, Agent], vec![], vec![Files, Agent]).is_some(), "左栏为空");
        assert!(go(vec![Files, Agent], vec![Files, Agent], vec![]).is_some(), "右栏为空");
        assert!(go(vec![Files, Agent], vec![Files], vec![Files, Agent]).is_some(), "默认布局里重复");
        assert!(go(vec![Files, Agent], vec![Files], vec![Web]).is_some(), "默认布局里有未注册面板");
        assert!(go(vec![Files, Agent, Todo], vec![Files], vec![Agent]).is_some(), "注册了却没放进默认布局");
    }

    #[test]
    fn migrate_legacy_is_the_composition_roots_hook() {
        fn to_default(_: RailLayout) -> RailLayout {
            rail(&[PanelKind::Files], &[PanelKind::Agent])
        }
        let c = PanelCatalog::new(
            vec![desc(PanelKind::Files), desc(PanelKind::Agent)],
            vec![PanelKind::Files],
            vec![PanelKind::Agent],
            to_default,
        )
        .unwrap();
        let out = c.migrate_legacy(rail(&[PanelKind::Web], &[]));
        assert_eq!(out, rail(&[PanelKind::Files], &[PanelKind::Agent]));
    }

    /// 全局清单只能装一次(第二次装返回错误,不覆盖)。测试里 `catalog()` 已自动装好 Dozer 清单。
    #[test]
    fn install_is_one_shot() {
        let _ = catalog();
        assert!(install(small()).is_err());
        assert_eq!(catalog().len(), 12, "没被覆盖");
    }

    /// `PanelKind` 新增变体时这里必须同步:穷举 match 没有通配,漏了编译不过。
    fn every_kind() -> Vec<PanelKind> {
        use PanelKind::*;
        let all = [
            Files, GitLog, Todo, Project, Database, Ssh, Web, Agent, GroupChat, Conversations,
            Usage, CodeHealth,
        ];
        for k in all {
            match k {
                Files | GitLog | Todo | Project | Database | Ssh | Web | Agent | GroupChat
                | Conversations | Usage | CodeHealth => {}
            }
        }
        all.to_vec()
    }

    #[test]
    fn dozer_catalog_registers_every_panel_kind_exactly_once() {
        let c = crate::product::dozer_catalog();
        assert_eq!(c.len(), every_kind().len());
        for k in every_kind() {
            assert!(c.descriptor(k).is_some(), "{k:?} 没注册");
            assert!(c.default_side(k).is_some(), "{k:?} 不在默认布局里");
        }
    }
}
```

再写骨架脚本 `$SCRATCH/h7_skeleton.py`(带引号 heredoc),运行并确认编译失败:

```python
#!/usr/bin/env python3
"""H7 一次性脚本(步骤 5):挂上两个新模块的骨架——`panel_registry.rs` 里只有测试、`product.rs` 是占位,
这样编译会因为缺类型而失败(RED)。`SP` 是 tests_*.rs 所在目录。**不提交。**"""
import os, shutil

src = "crates/dozer-app/src/"
shutil.copy(os.path.join(os.environ["SP"], "tests_panel_registry.rs"), src + "panel_registry.rs")
open(src + "product.rs", "w").write("//! (H7 占位,实现脚本会覆盖)\n")
s = open(src + "main.rs").read()
for a, b in (("mod panel_layouts;\n", "mod panel_layouts;\nmod panel_registry;\n"), ("mod preview_state;\n", "mod preview_state;\nmod product;\n")):
    assert a in s, a
    s = s.replace(a, b, 1)
open(src + "main.rs", "w").write(s)
print("ok")
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/h7 python3 $SCRATCH/h7_skeleton.py
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app --no-run 2>&1 | grep -E "^error" | sort | uniq -c; git checkout -- Cargo.lock
```
Expected: 脚本输出 `ok`;编译失败,报错包含 `cannot find type PanelDescriptor`、`cannot find type PanelCatalog`、`cannot find function catalog`/`install`、`cannot find function dozer_catalog in module crate::product`(RED)。

- [ ] **Step 5: 写实现脚本并运行**

创建 `$SCRATCH/h7_impl.py`(带引号 heredoc):

```python
#!/usr/bin/env python3
"""H7 一次性实现脚本:新增 `panel_registry.rs`(host 的清单类型 + 全局持有者)与 `product.rs`(Dozer 组合根),
`default_side`/`RailLayout::default`/`panel_meta`/`sanitize_rail_layout` 改走清单。**不提交。** 在仓库根运行。"""
SRC = "crates/dozer-app/src/"


def read(p):
    return open(SRC + p).read()


def write(p, s):
    open(SRC + p, "w").write(s)


def edit(p, pairs):
    s = read(p)
    for a, b in pairs:
        assert a in s, (p, a[:80])
        s = s.replace(a, b, 1)
    write(p, s)


# 1) panel_registry.rs:在已有的测试段之前插入实现
REGISTRY = '''//! 面板注册清单(bytehost H7):host 只认"清单"——有哪些面板、各自的标题/图标、默认放哪一栏、
//! 什么顺序——而清单本身由**产品组合根**(Dozer 见 `product.rs`)在启动时给出。
//!
//! 默认栏位与默认顺序属于产品(同一个面板在 Dozer 与 Digger 里的默认位置本来就不同),所以描述符
//! (`PanelDescriptor`)里只放面板自身的属性,默认布局是 `PanelCatalog` 的另一份输入。"恰好这些面板"
//! 的布局校验由清单驱动,不再写死"恰 12 个"。
//!
//! 本模块**不**做的事(留给 H8):`PanelKind` 仍是封闭枚举,每面板展开的状态字段与 `Message` 包装
//! 不动;切入钩子的分发(`fire_panel_switch_in`)仍是 host 的 `match`——各面板钩子的状态参数与消息
//! 类型各不相同,没有统一函数签名可以放进清单。

use std::sync::OnceLock;

use byteui::interaction::icons::IconKind;

use crate::app::{PanelKind, Side};
use crate::chrome::rail::RailLayout;

/// 一个面板自身的属性(不含默认栏位——那是产品的布局偏好)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PanelDescriptor {
    pub id: PanelKind,
    /// 图标栏 tooltip 文案。
    pub title: &'static str,
    pub icon: IconKind,
}

/// 产品给 host 的面板清单:描述符 + 默认布局 + 旧版落盘布局的迁移钩子。
pub(crate) struct PanelCatalog {
    descriptors: Vec<PanelDescriptor>,
    default_left: Vec<PanelKind>,
    default_right: Vec<PanelKind>,
    migrate_legacy: fn(RailLayout) -> RailLayout,
}

impl PanelCatalog {
    /// 校验后构造:描述符不重复且非空;默认布局两栏都非空、不重复、只含已注册面板,并且恰好覆盖全部
    /// 已注册面板(默认布局本身就是一个合法布局)。不满足返回说明原因的错误。
    pub(crate) fn new(
        descriptors: Vec<PanelDescriptor>,
        default_left: Vec<PanelKind>,
        default_right: Vec<PanelKind>,
        migrate_legacy: fn(RailLayout) -> RailLayout,
    ) -> Result<Self, String> {
        if descriptors.is_empty() {
            return Err("面板清单为空".into());
        }
        let mut ids: Vec<PanelKind> = descriptors.iter().map(|d| d.id).collect();
        ids.sort_by_key(|k| format!("{k:?}"));
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err("面板描述符里有重复的面板".into());
        }
        let catalog = Self {
            descriptors,
            default_left,
            default_right,
            migrate_legacy,
        };
        if !catalog.accepts(&catalog.default_rail()) {
            return Err(
                "默认布局不合法:两栏都要非空,且恰好覆盖全部已注册面板、不重复、不含未注册面板".into(),
            );
        }
        Ok(catalog)
    }

    /// 已注册的面板数。
    pub(crate) fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub(crate) fn descriptor(&self, id: PanelKind) -> Option<&PanelDescriptor> {
        self.descriptors.iter().find(|d| d.id == id)
    }

    /// 产品给的默认布局(两栏各自的面板与顺序)。
    pub(crate) fn default_rail(&self) -> RailLayout {
        RailLayout {
            left: self.default_left.clone(),
            right: self.default_right.clone(),
        }
    }

    /// 面板默认挂在哪条栏;没注册的面板返回 `None`。
    pub(crate) fn default_side(&self, id: PanelKind) -> Option<Side> {
        if self.default_left.contains(&id) {
            Some(Side::Left)
        } else if self.default_right.contains(&id) {
            Some(Side::Right)
        } else {
            None
        }
    }

    /// 旧版落盘布局的迁移(产品自己的历史,由组合根提供)。
    pub(crate) fn migrate_legacy(&self, rail: RailLayout) -> RailLayout {
        (self.migrate_legacy)(rail)
    }

    /// `rail` 是不是一个合法布局:两栏都非空,且恰好是全部已注册面板(每个一次,不多不少)。
    pub(crate) fn accepts(&self, rail: &RailLayout) -> bool {
        if rail.left.is_empty() || rail.right.is_empty() {
            return false;
        }
        let mut all: Vec<PanelKind> = rail.left.iter().chain(rail.right.iter()).copied().collect();
        all.sort_by_key(|k| format!("{k:?}"));
        if all.windows(2).any(|w| w[0] == w[1]) {
            return false;
        }
        all.len() == self.len() && all.iter().all(|k| self.descriptor(*k).is_some())
    }
}

static CATALOG: OnceLock<PanelCatalog> = OnceLock::new();

/// 启动时由组合根调用一次(`main()` 在任何布局读取之前)。已经装过则返回错误,不覆盖。
pub(crate) fn install(catalog: PanelCatalog) -> Result<(), &'static str> {
    CATALOG.set(catalog).map_err(|_| "面板清单已经注册过")
}

/// 当前产品的面板清单。生产里必须已 `install`;测试里自动装 Dozer 的清单。
pub(crate) fn catalog() -> &'static PanelCatalog {
    #[cfg(test)]
    {
        CATALOG.get_or_init(crate::product::dozer_catalog)
    }
    #[cfg(not(test))]
    {
        CATALOG
            .get()
            .expect("面板清单未注册:main() 必须在任何布局读取之前调用 panel_registry::install")
    }
}

'''
s = read("panel_registry.rs")
i = s.index("#[cfg(test)]")
# 文件开头的测试头注释保留在测试段之前
head = s[:s.index("#[cfg(test)]")]
write("panel_registry.rs", REGISTRY + s[i:])

# 2) product.rs:Dozer 组合根
write("product.rs", '''//! 产品组合根(bytehost H7):Dozer 的面板清单——有哪些面板、标题与图标、默认栏位与顺序、
//! 旧版落盘布局的迁移。host 只通过 `panel_registry` 认清单,不知道这里的具体面板;换一个产品
//! (如 Digger)就是另写一份这样的函数。

use byteui::interaction::icons::IconKind;

use crate::app::PanelKind;
use crate::chrome::rail::migrate_legacy_rail;
use crate::panel_registry::{PanelCatalog, PanelDescriptor};

pub(crate) fn dozer_catalog() -> PanelCatalog {
    use PanelKind::*;
    let d = |id, icon, title| PanelDescriptor { id, title, icon };
    PanelCatalog::new(
        vec![
            d(Files, IconKind::FolderTree, "文件"),
            d(GitLog, IconKind::GitGraph, "Git Log"),
            d(Todo, IconKind::ListTodo, "待办"),
            d(Project, IconKind::Briefcase, "项目"),
            d(Database, IconKind::Database, "数据库"),
            d(Ssh, IconKind::Server, "SSH 主机"),
            d(Web, IconKind::Globe, "浏览器"),
            d(Agent, IconKind::Brain, "代理"),
            d(GroupChat, IconKind::SquareSparkles, "群聊"),
            d(Conversations, IconKind::BotMessageSquare, "对话"),
            d(Usage, IconKind::BarChart3, "用量"),
            d(CodeHealth, IconKind::SquareActivity, "代码健康度"),
        ],
        vec![Project, Todo, Files, GitLog, Database, Ssh, Web],
        vec![Agent, GroupChat, Conversations, Usage, CodeHealth],
        migrate_legacy_rail,
    )
    .expect("Dozer 面板清单自检失败")
}
''')

# 3) main.rs:注册模块并在启动最前装清单
edit("main.rs", [
    ("""    // 第一次文本排版之前：先注册内嵌的 JetBrains Mono""", """    // 产品组合根:先装面板清单,之后任何布局读取/校验都以它为准。
    panel_registry::install(product::dozer_catalog()).expect("面板清单只注册一次");

    // 第一次文本排版之前：先注册内嵌的 JetBrains Mono"""),
])

# 4) default_side / Default / panel_meta / sanitize 改走清单
edit("app/state.rs", [(
"""    pub fn default_side(self) -> Side {
        match self {
            Self::Files
            | Self::GitLog
            | Self::Todo
            | Self::Project
            | Self::Database
            | Self::Ssh
            | Self::Web => Side::Left,
            Self::Agent
            | Self::GroupChat
            | Self::Conversations
            | Self::Usage
            | Self::CodeHealth => Side::Right,
        }
    }""",
"""    pub fn default_side(self) -> Side {
        crate::panel_registry::catalog()
            .default_side(self)
            .unwrap_or_else(|| panic!("{self:?} 没有在面板清单里注册"))
    }""")])

edit("chrome/rail.rs", [
("""impl Default for RailLayout {
    fn default() -> Self {
        Self {
            left: vec![
                PanelKind::Project,
                PanelKind::Todo,
                PanelKind::Files,
                PanelKind::GitLog,
                PanelKind::Database,
                PanelKind::Ssh,
                PanelKind::Web,
            ],
            right: vec![
                PanelKind::Agent,
                PanelKind::GroupChat,
                PanelKind::Conversations,
                PanelKind::Usage,
                PanelKind::CodeHealth,
            ],
        }
    }
}""",
"""impl Default for RailLayout {
    /// 产品组合根给的默认布局(见 `product::dozer_catalog`)。
    fn default() -> Self {
        crate::panel_registry::catalog().default_rail()
    }
}"""),
("""/// `RailLayout` 的消毒:先做旧布局迁移;然后任一栏为空,或两侧合计不是恰 12 个
/// 不重复的 `PanelKind`(手改/版本不一致导致的坏数据),整个回落 `default()`。""",
"""/// `RailLayout` 的消毒:先做旧布局迁移;然后任一栏为空,或两侧合计不是恰好等于面板清单里
/// 那些面板各一次(手改/版本不一致导致的坏数据),整个回落 `default()`。"""),
("""    let rail = migrate_legacy_rail(rail);
    if rail.left.is_empty() || rail.right.is_empty() {
        return RailLayout::default();
    }
    let mut all: Vec<_> = rail.left.iter().chain(rail.right.iter()).collect();
    all.sort_by_key(|k| format!("{k:?}"));
    all.dedup();
    if all.len() != 12 || rail.left.len() + rail.right.len() != 12 {
        return RailLayout::default();
    }
    rail
}""",
"""    let catalog = crate::panel_registry::catalog();
    let rail = catalog.migrate_legacy(rail);
    if !catalog.accepts(&rail) {
        return RailLayout::default();
    }
    rail
}"""),
("""/// 面板 → (图标, 图标栏 tooltip 文案)。12 个 `PanelKind` variant 逐一
/// 对应,顺序与 `PanelKind` 定义顺序一致,不代表渲染顺序(渲染顺序看
/// `RailLayout`)。
fn panel_meta(kind: PanelKind) -> (icons::IconKind, &'static str) {
    match kind {
        PanelKind::Files => (icons::IconKind::FolderTree, "文件"),
        PanelKind::GitLog => (icons::IconKind::GitGraph, "Git Log"),
        PanelKind::Todo => (icons::IconKind::ListTodo, "待办"),
        PanelKind::Project => (icons::IconKind::Briefcase, "项目"),
        PanelKind::Database => (icons::IconKind::Database, "数据库"),
        PanelKind::Ssh => (icons::IconKind::Server, "SSH 主机"),
        PanelKind::Web => (icons::IconKind::Globe, "浏览器"),
        PanelKind::Agent => (icons::IconKind::Brain, "代理"),
        PanelKind::GroupChat => (icons::IconKind::SquareSparkles, "群聊"),
        PanelKind::Conversations => (icons::IconKind::BotMessageSquare, "对话"),
        PanelKind::Usage => (icons::IconKind::BarChart3, "用量"),
        PanelKind::CodeHealth => (icons::IconKind::SquareActivity, "代码健康度"),
    }
}""",
"""/// 面板 → (图标, 图标栏 tooltip 文案),取自面板清单(见 `product::dozer_catalog`)。
fn panel_meta(kind: PanelKind) -> (icons::IconKind, &'static str) {
    let d = crate::panel_registry::catalog()
        .descriptor(kind)
        .unwrap_or_else(|| panic!("{kind:?} 没有在面板清单里注册"));
    (d.icon, d.title)
}"""),
])
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7 && export PYTHONDONTWRITEBYTECODE=1
python3 $SCRATCH/h7_impl.py && cargo fmt -p dozer-app && git status --short
```
Expected: `git status --short` 是 `app/state.rs`、`chrome/rail.rs`、`main.rs`(已修改),`panel_registry.rs`、`product.rs`(新文件,`??`)(和 `Cargo.lock`,马上还原)。脚本里每个替换点都先 `assert` 原文存在;若报 `AssertionError`,说明 `main` 上这几处已被改动,按报错点手工对齐后再继续,**不要**改断言。

- [ ] **Step 6: 编译、全量测试、clippy、门禁**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7
RUSTFLAGS="-D unconditional_recursion" cargo build -p dozer-app 2>&1 | grep -E "^(error|warning)" -A6 | grep -E "panel_registry|product|^error" | head
RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|^test result|FAILED"
git checkout -- Cargo.lock
RUSTFLAGS="-D unconditional_recursion" cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "^(warning|error)" -A4 | grep -E "^\s+--> " | sed 's/:[0-9]*:[0-9]*$//' | sort | uniq -c > $SCRATCH/clippy-after.txt; git checkout -- Cargo.lock
diff $SCRATCH/clippy-before.txt $SCRATCH/clippy-after.txt && echo "clippy: no new diagnostics"
python3 scripts/audit/check_panel_boundary.py
```
Expected: build 里没有指向 `panel_registry`/`product` 的警告或错误;测试通过数 = N + 10,失败只有已知项(`assets::…serves_vendored_asset_with_mime` 若偶发失败,重跑一次);`clippy: no new diagnostics`;门禁 `ok (122 refs …)`。

- [ ] **Step 7: 审阅 `main.rs` 与其余 host 侧 diff(Review Focus 1)**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7 && git diff -- crates/dozer-app/src/main.rs crates/dozer-app/src/app/state.rs crates/dozer-app/src/chrome/rail.rs | grep -E '^[-+]' | grep -vE '^(\+\+\+|---)'; grep -rn "RailLayout::default()\|default_side()" crates/dozer-app/src --include=*.rs | grep -v "mod tests" | grep -vE "^crates/dozer-app/src/(chrome/rail.rs|app/state.rs)" | head -20`
Expected(逐条对照 Review Focus 1、3):
1. `main.rs`:两行 `mod`,以及 `panel_registry::install(product::dozer_catalog()).expect(…)` 紧接在 `dozer_core::log::init(…)` 之后、`load_embedded_fonts()` 之前;
2. `state.rs`:`default_side` 的 12 臂 match 换成 `catalog().default_side(self).unwrap_or_else(|| panic!(…))`;
3. `rail.rs`:`Default for RailLayout` 一行取 `default_rail()`;`panel_meta` 取描述符的 `(icon, title)`;`sanitize_rail_layout` 三步(迁移钩子 → `accepts` → 回落默认)与旧的"空栏/12 个不重复"判定一致;`migrate_legacy_rail` **本身没改**;
4. 第二条命令列出的是**其他文件**里调用 `RailLayout::default()`/`default_side()` 的生产代码——它们都在 `install` 之后才可能被调用(都发生在 `Runner`/事件循环建立之后),确认没有出现在 `main()` 之前执行的路径上。

- [ ] **Step 8: 变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7 && export PYTHONDONTWRITEBYTECODE=1
git add -A crates
cat > $SCRATCH/h7_mutate.py <<'EOF'
import sys
w = sys.argv[1]
S = "crates/dozer-app/src/"
if w == "1":   # 默认顺序:Project/Todo 互换
    p, a, b = S + "product.rs", "vec![Project, Todo, Files, GitLog, Database, Ssh, Web],", "vec![Todo, Project, Files, GitLog, Database, Ssh, Web],"
elif w == "2":   # 某个面板图标写错
    p, a, b = S + "product.rs", 'd(Web, IconKind::Globe, "浏览器"),', 'd(Web, IconKind::Server, "浏览器"),'
elif w == "3":   # accepts 不再挡重复
    p, a = S + "panel_registry.rs", "        if all.windows(2).any(|w| w[0] == w[1]) {\n            return false;\n        }\n"
    b = ""
elif w == "4":   # sanitize 不再先跑迁移钩子
    p, a, b = S + "chrome/rail.rs", "let rail = catalog.migrate_legacy(rail);", "let rail = rail;"
elif w == "5":   # default_side 左右颠倒
    p, a, b = S + "panel_registry.rs", "        if self.default_left.contains(&id) {\n            Some(Side::Left)", "        if self.default_left.contains(&id) {\n            Some(Side::Right)"
elif w == "6":   # 清单校验不再要求默认布局覆盖全部已注册面板
    p, a, b = S + "panel_registry.rs", "        if !catalog.accepts(&catalog.default_rail()) {", "        if false {"
s = open(p).read()
assert a in s, w
open(p, "w").write(s.replace(a, b, 1))
EOF
for m in 1 2 3 4 5 6; do echo "M$m:"; python3 $SCRATCH/h7_mutate.py $m && RUSTFLAGS="-D unconditional_recursion" cargo test -p dozer-app 2>&1 | grep -E "^error|FAILED$" | grep -v "delete_confirm\|serves_vendored" | head -4; git checkout -- crates Cargo.lock; done
git diff --stat | wc -l; git status --short
```
Expected: 六次变异各自让测试失败(除了永远失败的 `delete_confirm` 与偶发的 `serves_vendored`):M1(默认顺序 Project/Todo 互换):`golden_default_layout_order_and_sides` 等;M2(Web 图标写错):`golden_panel_icons_and_titles`;M3(`accepts` 不再挡重复):`panel_registry::tests::accepts_exactly_the_registered_panels_in_any_arrangement`;M4(`sanitize` 不跑迁移钩子):`legacy_*` 系列测试;M5(`default_side` 左右颠倒):一批 `apply_column_drag_*` 与 `golden_default_layout_order_and_sides`;M6(清单构造不再校验默认布局):`panel_registry::tests::new_rejects_inconsistent_catalogs`;每次还原后 `git diff --stat | wc -l` 为 `0`,`git status --short` 仍是 Step 5 的 5 个条目。

- [ ] **Step 9: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7
git status --short
git add crates
git diff --cached --stat | tail -8
git commit -m "refactor(panels): PanelCatalog — the composition root registers the panel list, default sides/order, icons and titles

Adds panel_registry (PanelDescriptor, PanelCatalog with construction-time validation, install/catalog)
and product::dozer_catalog() as Dozer's composition root. PanelKind::default_side, RailLayout::default,
the rail's panel_meta and sanitize_rail_layout's exact-set check now read the catalog instead of
hard-coding the 12 panels. Persisted format unchanged (golden tests added). PanelKind stays a closed
enum; PanelKind->PanelId and hooks-in-catalog are left to H7b/H8.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `git diff --cached --stat` 里只有 6 个 `crates/` 文件(没有 `Cargo.lock`、`docs/`、`scripts/`)。

---

### Task 2: 文档回填

**Files:**
- Modify: `docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md`、`docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`、`docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md`

**Interfaces:**
- Consumes: Task 1 的提交。

- [ ] **Step 1: 回填并校验**

用带引号的 heredoc 创建 `$SCRATCH/h7_docs.py`(提交短 id 经环境变量传入):

```python
import os
H7 = os.environ["H7"]
sp = "docs/superpowers/specs/2026-10-04-bytehost-panel-hooks-and-registry-design.md"
s = open(sp).read()
a = "## 4. 迁移顺序"
assert a in s
note = "(H7a 实现取舍:先落地清单——`panel_registry::PanelCatalog`(描述符 + 默认布局 + 旧布局迁移钩子,构造时校验)与 Dozer 组合根 `product::dozer_catalog()`;默认栏位/顺序/标题/图标/\"恰为清单\"校验改走清单。**`PanelKind` 仍是封闭枚举**:把它换成 `PanelId` 约 1150 行引用的纯类型替换,在 H8 之前没有任何收益(每面板的状态字段与消息包装仍点名 12 个面板),列为 H7b、随 H8 评估;`PanelDescriptor` 里**没有 `hooks`**:各面板钩子的状态参数与消息类型各不相同,没有统一签名可放进清单,`fire_panel_switch_in` 仍是 host 的 `match`,同样随 H8。)\n\n"
s = s.replace(a, note + a, 1)
lines = s.split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("| **H7** |"):
        lines[k] = l[:-1].rstrip() + " **部分完成(H7a,`bytehost-h7`):`" + H7 + "`**——清单类型 + Dozer 组合根;默认栏位/顺序、图标/标题、布局校验改走清单;`PanelId` 类型替换与 hooks 入清单留给 H7b/H8 |"
        hit = True
assert hit
open(sp, "w").write("\n".join(lines))
bp = "docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md"
s = open(bp).read()
lines = s.split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("| O5 |"):
        lines[k] = l[:-1].rstrip() + " **已定(2026-10-04,用户):产品组合根决定;H7a 落地(`panel_registry`/`product`)** |"
        hit = True
assert hit
open(bp, "w").write("\n".join(lines))
rp = "docs/dozer-v2/dozer-v2-面板独立性与服务化要求.md"
s = open(rp).read().rstrip("\n")
s += "\n\n12. **bytehost H7a 已完成（2026-10-04）：** 引入 `PanelCatalog`（描述符 + 默认布局 + 旧布局迁移钩子，构造时校验）与 Dozer 组合根 `product::dozer_catalog()`；`PanelKind::default_side()`、`RailLayout::default()`、图标栏的 `panel_meta`、`sanitize_rail_layout` 的“恰 12 个”校验不再写死面板名单，改由清单驱动（O5 落地）。落盘格式不变（`PanelKind` 仍序列化为枚举名，已有黄金测试）。`PanelKind` 换成 `PanelId`、把切入钩子放进清单两项留给 H7b/H8，理由见设计文档 §3.4 补注。\n"
open(rp, "w").write(s)
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7 && export PYTHONDONTWRITEBYTECODE=1
H7=$(git log --format=%h -1 --grep="PanelCatalog") python3 $SCRATCH/h7_docs.py
git diff -- docs | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)" | cut -c1-170
python3 scripts/audit/check_panel_boundary.py
```
Expected: `git diff` 里只有:设计文档 §4 前的 H7a 补注与 H7 行、边界设计文档 O5 行、要求文档第 12 条,**反引号内容完整**(若有被吞掉的痕迹,说明用了不带引号的 heredoc——`git checkout -- docs` 后重做);门禁 `ok`。

- [ ] **Step 2: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-h7
git status --short
git add docs
git diff --cached --stat | tail -5
git commit -m "docs(bytehost): backfill H7a (panel catalog and Dozer composition root)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 只有 `docs/` 下 3 个文件。

---

## Self-Review

**1. 覆盖:** 设计文档 §3.4/§4 H7 的"描述符/registry + 组合根给默认栏位 + 布局校验按清单 + serde 兼容测试"——全部有对应(清单类型、`product.rs`、`accepts`、4 个黄金测试含 JSON 兼容与旧 11 面板迁移)。**未覆盖并已在"细化"里说明理由:** `PanelId` 类型替换(H7b)、描述符里的 `hooks`/`needs_preview_column`。**坦白收益:** 本刀不减少 host 里点名面板的引用行数(`PanelKind` 仍是枚举),减少的是"写死的面板名单"——默认栏位 12 臂 match、`RailLayout::default` 的字面列表、`panel_meta` 的 12 臂 match、校验里的魔数 12——并让"只有 3 个面板的产品"在清单层面成为可能(测试已证明)。

**2. 占位符扫描:** 测试、骨架脚本、实现脚本、变异脚本、文档脚本均为草稿里跑通的完整版本(特征测试在旧代码上先过;六个变异均被抓到;clippy 与基线逐文件一致)。骨架脚本是把草稿里手工做的"挂模块 + 复制测试文件 + 建占位 product.rs"固化,逻辑等价。

**3. 一致性:** `PanelCatalog::{new, len, descriptor, default_rail, default_side, migrate_legacy, accepts}`、`install`/`catalog`、`product::dozer_catalog` 的名字与形状在测试、实现、Interfaces、文档里一致。

**4. Review Focus:** 6 条各有归属(1→Step 7 读 diff;2→穷举 `match` 测试 + 说明;3→特征测试 + Step 6/8;4→说明 + 既有测试;5→`install_is_one_shot`;6→范围说明)。
