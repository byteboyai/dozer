# bytehost 应用宿主 A0:新建 `bytehost-apps` crate(应用模型、安装计划、生命周期、注册表——无界面部分的类型与纯逻辑) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地应用宿主规格(`docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`)§3.2、§4、§7 的 **A0** 切片:新建 `crates/bytehost-apps`,放进应用宿主**不需要异步、不需要网络、不需要界面**的全部部分——应用 id/版本、manifest(TOML)与校验、权限模型与差异、安装计划与"审批绑定摘要"、生命周期的 desired/observed 对账纯函数、事件类型、注册表与磁盘布局。**不碰任何现有 crate 的代码**(除了 `Cargo.lock` 里多出新 crate 的 11 行与 `CLAUDE.md` 的 crate 表)。

**Architecture:** 一个代码任务 + 一个文档任务。Task 1 测试先行:先写 8 个模块的测试文件并建骨架(编译失败,RED),再跑装配脚本写入实现,逐个 feature 组合测试、clippy、依赖门禁,9 个变异检验证明测试会失败。Task 2 回填文档(规格、路线图、`CLAUDE.md` 的 crate 表)。

**Tech Stack:** Rust(新 crate `bytehost-apps`,edition 2024);依赖:默认 `serde`、`serde_json`;`manifest-toml` feature → `toml 0.8`;`digest` feature → `sha2 0.10`;dev-dependency `tempfile`。这些版本在 `Cargo.lock` 里都已存在。Python 3 标准库(一次性脚本)。

**Spec:** 应用宿主规格 §3.2(crate 与依赖方向)、§4.1–§4.6(模型)、§7(切片 A0);路线图 `2026-10-04-bytehost-extraction-roadmap.md` §3 阶段 1。**用户裁决:** manifest 用 TOML(2026-10-04);crate 名 `bytehost-apps`(A10)。

## Global Constraints

- **crate 不得依赖任何 `dozer*` crate,默认 feature 的依赖闭包只能是 serde/serde_json 家族。** 原因:它要被 Digger 复用;且 `dozer-core` 将来会依赖它,`dozer-hook`/`dozer-mcp` 的依赖闭包不能因此多出新 crate。由 `scripts/check-bytehost-apps-deps.sh` 守住(本计划新增,Task 1 里还会做两个变异证明它真的会拦)。
- **本 crate 里不得出现 `dozer`/`Dozer` 字样的类型或路径**(规格 §3.2)。
- **在专用 worktree 里执行,路径写全**(`~/Projects/CoralProjects/byteboy/dozer-bytehost-a0/...`),分支 `bytehost-a0`,从当前 `main` 开。主 checkout 常有并发会话与未提交改动,不得在其上改代码或 `git add -A`;只 `git add` 指定路径;提交前 `git status` 与 `git diff --cached --stat` 看全貌。
- **这次 worktree 不要复制 `.cargo/config.toml`。** 本计划不需要联调本地 byteui/bytegit;不带 `[patch]` 时 `Cargo.lock` 保持干净,新 crate 只会让它多 11 行,可以**直接提交**(与前几个切片"每次提交前还原 `Cargo.lock`"相反)。提交前用 `git diff Cargo.lock` 确认只有 `bytehost-apps` 这一个 `[[package]]` 块的新增,没有任何 `source = …` 行的变化。
- **变异检验前先 `git add` 暂存当前版本**(新文件也要 `git add`),变异后用 `git checkout -- crates` 还原。
- **每次改 Rust 后跑 `cargo fmt -p bytehost-apps`;变异脚本匹配的是 `cargo fmt` 之后的源码。**
- **heredoc 一律用带引号的分隔符(`<<'EOF'`);或直接用编辑器/Write 工具写文件。**
- 所有测试都在各模块自己的 `#[cfg(test)] mod tests` 里(与现有 crate 的风格一致)。
- 提交信息用 `feat:`/`test:`/`docs:` 前缀;末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## 已知基线

- 本切片不改任何现有代码,**全 workspace 的测试结果不变**;只需要证明新 crate 自身的测试数与不同 feature 组合下的行为。
- 草稿结果:`cargo test -p bytehost-apps`(默认 feature)**47 passed**;`--features digest` 53;`--features manifest-toml` 53;`--all-features` **59**。clippy 在默认与 `--all-features` 下均**零诊断**。

## Review Focus

1. **`ApprovedInstallPlan` 可以从 JSON 反序列化出来(为了走线上协议),所以"构造只能经 `approve`"只是类型层面的约束,不是安全边界。** 真正的安全属性是 `verify`:安装时用**重新计算**的两个摘要核对。审批本身的真实性(是不是真的有用户点了确认)是产品/supervisor 与客户端之间的信任问题(同一条 UDS 上的其他请求同级),不在 A0 范围——审阅时确认没有任何注释或文档把 `Approval` 说成"防伪"。
2. **`digest_tree` 不包含文件权限位(可执行位)**:对 `static_web` 没有影响;对 Node/Python/容器的脚本,"审批后把 `run.sh` 改成可执行"不会改变摘要。这是已知局限,A1 的进程型 runtime 落地前要决定是否把 mode 纳入摘要。另外空目录不计入摘要,符号链接直接报错(链接可以指向包外)。
3. **`next_action` 的顺序语义:** `desired = Stopped/Removed` 且观察态是 `Preparing/Starting` 时给 `Stop`(取消进行中的动作),其余过渡态一律"等";`Removed` + `Running` 先 `Stop`,下一轮对账再 `Uninstall`。11 种观察态 × 3 种 desired 的完整真值表由测试固定。
4. **`recover_after_supervisor_restart` 的前提是规格 §6.1:应用跟随 dozerd,dozerd 停止则应用停止。** 所以"曾在运行/过渡中"的一律回到 `Stopped`。如果将来改成 supervisor 与应用解耦(Digger 的实现可能不同),这个函数要换成"探测后再定",而不是直接映射。
5. **`Registry` 的持久性假设:** 原子写是"写临时文件再改名",**没有 `fsync`**(断电可能丢最后一次写);**不处理并发写者**(规格里 supervisor 是唯一写者);`uninstall(Program)` 先删包、缓存、manifest,**最后**删 `state.json`——中途崩溃会留下"有 state.json、没有包"的状态,这要靠 A1 的对账处理(本切片只保证"删除顺序使 `list` 不会把半删的应用当成干净卸载")。
6. **`AppId` 允许 `xn--` 前缀等合法主机名字符**——它会成为 `<id>.localhost` 的主机名;本切片不做保留名/IDN 过滤(没有已知风险,A1 的 gateway 要对 Host 做严格匹配)。
7. **`Manifest` 用 `deny_unknown_fields`,包括内部带 `kind` 标签的 `Runtime` 枚举和嵌套的权限表**——测试 `unknown_fields_are_a_parse_error_at_the_top_level_and_in_nested_tables` 三层都覆盖;`toml` 解析器若在升级后改变对"标签枚举里的未知字段"的处理,这条测试会先失败。
8. **范围外(明确不做):** 线上协议类型(`AppRequest`/`AppResponse`,A2)、`AppManager`、runtime adapter、gateway(A1)、与 `dozer-core`/`dozerd` 的任何接线(A2)、GUI(A4)。

---

## File Structure

| 路径 | 职责 | 任务 |
|---|---|---|
| `crates/bytehost-apps/Cargo.toml`(新) | features:默认空、`manifest-toml`、`digest` | 1 |
| `crates/bytehost-apps/src/lib.rs`(新) | 模块声明与 feature 说明 | 1 |
| `crates/bytehost-apps/src/id.rs`(新) | `AppId`、`Version` + 6 个测试 | 1 |
| `crates/bytehost-apps/src/permissions.rs`(新) | 权限/强制等级/差异 + 7 个测试 | 1 |
| `crates/bytehost-apps/src/manifest.rs`(新) | manifest 类型、校验、`from_toml` + 15 个测试 | 1 |
| `crates/bytehost-apps/src/digest.rs`(新,feature `digest`) | SHA-256、目录摘要、数据存储标识 + 6 个测试 | 1 |
| `crates/bytehost-apps/src/plan.rs`(新) | 安装计划、审批、`verify` + 6 个测试 | 1 |
| `crates/bytehost-apps/src/state.rs`(新) | desired/observed、`next_action`、重启修正 + 7 个测试 | 1 |
| `crates/bytehost-apps/src/event.rs`(新) | `AppEvent`、`manifest_changed` + 3 个测试 | 1 |
| `crates/bytehost-apps/src/registry.rs`(新) | 磁盘布局、`AppRecord`、`Registry` + 9 个测试 | 1 |
| `scripts/check-bytehost-apps-deps.sh`(新) | 依赖门禁 | 1 |
| `Cargo.lock` | 多出 `bytehost-apps` 一个 `[[package]]` 块(11 行) | 1 |
| `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`、`…-extraction-roadmap.md`、`CLAUDE.md` | 回填 | 2 |

一次性脚本与测试/实现源文件(**不提交**,最终内容在 crate 里)放 `$SCRATCH`(仓库外)。

---

### Task 1: `bytehost-apps` crate

**Files:** 见上表(`crates/bytehost-apps/` 下 10 个新文件、`scripts/check-bytehost-apps-deps.sh`、`Cargo.lock`)。

**Interfaces:**
- Produces(A1/A2/A4 依赖),均 `pub`,全部在 crate 根的模块下:
  - `id::{AppId, IdError, Version, VersionError}`——`AppId::new(impl Into<String>) -> Result<AppId, IdError>`、`as_str()`;`Version::{new(u32,u32,u32), parse(&str)}`(`Ord`);两者都按字符串 serde。
  - `permissions::{Permissions { network: NetworkPerm, filesystem: FilesystemPerm, clipboard: Access, downloads: Gate, popups: Gate }, Access, Gate, Outbound, PermissionKey, Enforcement, PermissionChange, diff_permissions(&Permissions /*granted*/, &Permissions /*requested*/) -> Vec<PermissionChange>}`。
  - `manifest::{Manifest, Presentation, Entrypoint, EntrypointKind, Runtime, ProcessHttp, ContainerHttp, Health, ManifestError, SCHEMA_VERSION}`;`Manifest::validate(&self, &Version) -> Result<(), Vec<String>>`;`Manifest::from_toml(&str, &Version) -> Result<Manifest, ManifestError>`(feature `manifest-toml`);`Runtime::kind_name()`。
  - `digest::{sha256_hex(&[u8]) -> String, digest_tree(&Path) -> io::Result<String>, data_store_id(&AppId) -> [u8;16], data_store_id_hex(&AppId) -> String}`(feature `digest`)。
  - `plan::{InstallPlan, PlanInput, Installed, Provenance, TrustLevel, EnforcementEntry, Approval, ApprovedInstallPlan, VerifyError}`;`InstallPlan::build(PlanInput) -> InstallPlan`、`InstallPlan::approve(self, Approval) -> ApprovedInstallPlan`、`ApprovedInstallPlan::{plan(), approval(), verify(&str, &str) -> Result<&InstallPlan, VerifyError>}`。
  - `state::{DesiredState, ObservedState, Action, next_action(DesiredState, &ObservedState) -> Option<Action>, recover_after_supervisor_restart(ObservedState) -> ObservedState}`;`ObservedState::is_transient()`。
  - `event::{AppEvent, TaskId, Progress, RuntimeReason, manifest_changed(&Manifest, &Manifest) -> Option<AppEvent>}`;`AppEvent::app()`。
  - `registry::{AppPaths, AppRecord, VersionRecord, UninstallMode, Listing, Registry}`;`Registry::{open, paths, list, load, save, save_manifest, uninstall}`。

- [ ] **Step 1: 建 worktree、分支、脚本目录**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer
git worktree add ../dozer-bytehost-a0 -b bytehost-a0 main
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0 && git log --oneline | head -1 && git status --short && ls .cargo 2>/dev/null
export SCRATCH=~/Projects/CoralProjects/byteboy/dozer-bytehost-a0-scratch && mkdir -p $SCRATCH/a0
```

Expected: 干净、分支 `bytehost-a0`、`ls .cargo` 没有输出(**不要**复制 `config.toml`)。**此后所有命令都用 `~/Projects/CoralProjects/byteboy/dozer-bytehost-a0/` 前缀,且每个新 shell 先 `export SCRATCH=…`。**

- [ ] **Step 2: 确认起点**

Run: `cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0 && ls crates && cargo metadata --no-deps --format-version 1 >/dev/null && echo metadata-ok`
Expected: `crates` 里**没有** `bytehost-apps`;`metadata-ok`。

- [ ] **Step 3: 写测试与骨架文件(先看编译失败)**

把下面 8 段测试写成 `$SCRATCH/a0/tests_<模块>.rs`(每段开头有一个空行,保留);再写 `$SCRATCH/a0/Cargo.toml` 与 `$SCRATCH/a0/lib.rs`:

`$SCRATCH/a0/tests_id.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_accepts_lowercase_digits_and_inner_hyphens() {
        for ok in ["excalidraw", "a", "my-app-2", "0day", &"a".repeat(63)] {
            assert!(AppId::new(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn app_id_rejects_everything_that_could_escape_a_hostname_or_a_directory() {
        assert_eq!(AppId::new(""), Err(IdError::Empty));
        assert_eq!(AppId::new("a".repeat(64)), Err(IdError::TooLong));
        assert_eq!(AppId::new("-a"), Err(IdError::EdgeHyphen));
        assert_eq!(AppId::new("a-"), Err(IdError::EdgeHyphen));
        for bad in ["Excal", "a_b", "a.b", "a/b", "../x", "a b", "中文", "a:80"] {
            assert!(matches!(AppId::new(bad), Err(IdError::BadChar(_))), "{bad}");
        }
    }

    #[test]
    fn app_id_serde_round_trips_as_a_plain_string_and_validates_on_read() {
        let id = AppId::new("excalidraw").unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"excalidraw\"");
        assert_eq!(serde_json::from_str::<AppId>("\"excalidraw\"").unwrap(), id);
        assert!(serde_json::from_str::<AppId>("\"../etc\"").is_err());
    }

    #[test]
    fn version_parses_three_numeric_parts_only() {
        assert_eq!(Version::parse("0.17.3").unwrap(), Version::new(0, 17, 3));
        for bad in ["", "1", "1.2", "1.2.3.4", "1.2.x", "1..3", "v1.2.3", "1.2.3-rc1", "-1.2.3", "1.2.99999999999"] {
            assert!(Version::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn version_orders_numerically_not_lexically() {
        assert!(Version::new(0, 10, 0) > Version::new(0, 9, 0));
        assert!(Version::new(1, 0, 0) > Version::new(0, 99, 99));
        assert_eq!(Version::new(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn version_serde_round_trips_as_a_string() {
        let v = Version::new(0, 17, 0);
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"0.17.0\"");
        assert_eq!(serde_json::from_str::<Version>("\"0.17.0\"").unwrap(), v);
        assert!(serde_json::from_str::<Version>("\"0.17\"").is_err());
    }
}
```

`$SCRATCH/a0/tests_permissions.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_permissions_are_the_most_restrictive() {
        let p = Permissions::default();
        for key in PermissionKey::ALL {
            assert_eq!(p.level(key).0, 0, "{key:?}");
        }
    }

    #[test]
    fn unknown_permission_fields_are_rejected_at_every_level() {
        assert!(serde_json::from_str::<Permissions>(r#"{"telepathy":"allow"}"#).is_err());
        assert!(serde_json::from_str::<Permissions>(r#"{"network":{"outbound":"none","inbound":"any"}}"#).is_err());
        assert!(serde_json::from_str::<Permissions>(r#"{"filesystem":{"home":"read"}}"#).is_err());
        assert!(serde_json::from_str::<Permissions>(r#"{"clipboard":"sometimes"}"#).is_err());
    }

    #[test]
    fn permissions_serde_uses_snake_case_labels_and_fills_missing_with_strictest() {
        let p: Permissions = serde_json::from_str(r#"{"clipboard":"read_write","downloads":"user_confirm"}"#).unwrap();
        assert_eq!(p.clipboard, Access::ReadWrite);
        assert_eq!(p.downloads, Gate::UserConfirm);
        assert_eq!(p.popups, Gate::Deny);
        assert_eq!(p.network.outbound, Outbound::None);
    }

    #[test]
    fn levels_are_ordered_from_strict_to_loose() {
        assert!(Access::None < Access::Read && Access::Read < Access::ReadWrite);
        assert!(Gate::Deny < Gate::UserConfirm && Gate::UserConfirm < Gate::Allow);
        assert!(Outbound::None < Outbound::Any);
    }

    #[test]
    fn fresh_install_diff_lists_every_requested_capability_as_an_escalation() {
        let requested = Permissions {
            network: NetworkPerm { outbound: Outbound::Any },
            filesystem: FilesystemPerm { data: Access::ReadWrite },
            clipboard: Access::Read,
            downloads: Gate::UserConfirm,
            popups: Gate::Deny,
        };
        let diff = diff_permissions(&Permissions::default(), &requested);
        let keys: Vec<_> = diff.iter().map(|c| c.key).collect();
        assert_eq!(
            keys,
            vec![
                PermissionKey::NetworkOutbound,
                PermissionKey::FilesystemData,
                PermissionKey::Clipboard,
                PermissionKey::Downloads
            ],
            "popups 申请的就是 deny,和默认相同,不算差异;顺序固定"
        );
        assert!(diff.iter().all(|c| c.escalation));
        assert_eq!(diff[1].from, "none");
        assert_eq!(diff[1].to, "read_write");
    }

    #[test]
    fn upgrade_diff_distinguishes_escalation_from_reduction() {
        let granted = Permissions {
            clipboard: Access::ReadWrite,
            downloads: Gate::Allow,
            ..Permissions::default()
        };
        let requested = Permissions {
            clipboard: Access::Read,
            downloads: Gate::Allow,
            popups: Gate::UserConfirm,
            ..Permissions::default()
        };
        let diff = diff_permissions(&granted, &requested);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff[0].key, PermissionKey::Clipboard);
        assert!(!diff[0].escalation, "read_write → read 是降权");
        assert_eq!(diff[1].key, PermissionKey::Popups);
        assert!(diff[1].escalation);
    }

    #[test]
    fn identical_permissions_have_no_diff() {
        let p = Permissions { clipboard: Access::Read, ..Permissions::default() };
        assert!(diff_permissions(&p, &p).is_empty());
    }
}
```

`$SCRATCH/a0/tests_manifest.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: Version = Version::new(0, 1, 0);

    fn valid() -> Manifest {
        Manifest {
            schema_version: 1,
            min_host_version: Version::new(0, 1, 0),
            id: AppId::new("excalidraw").unwrap(),
            name: "Excalidraw".into(),
            version: Version::new(0, 17, 0),
            presentation: Presentation {
                icon: Some("assets/icon.svg".into()),
                surface_hint: Some("browser".into()),
                entrypoint: "main".into(),
            },
            entrypoints: BTreeMap::from([(
                "main".to_string(),
                Entrypoint { kind: EntrypointKind::Web, path: "/".into(), title: Some("Excalidraw".into()) },
            )]),
            runtime: Runtime::StaticWeb { source: "web/".into() },
            permissions: Permissions::default(),
            health: Health::default(),
        }
    }

    fn problems(m: &Manifest) -> Vec<String> {
        m.validate(&HOST).err().unwrap_or_default()
    }

    #[test]
    fn a_valid_static_web_manifest_passes() {
        assert_eq!(valid().validate(&HOST), Ok(()));
    }

    #[test]
    fn host_older_than_min_host_version_is_rejected() {
        let mut m = valid();
        m.min_host_version = Version::new(0, 2, 0);
        let p = problems(&m);
        assert_eq!(p.len(), 1);
        assert!(p[0].contains("0.2.0") && p[0].contains("0.1.0"), "{p:?}");
    }

    #[test]
    fn wrong_schema_version_is_rejected() {
        let mut m = valid();
        m.schema_version = 2;
        assert_eq!(problems(&m).len(), 1);
    }

    #[test]
    fn entrypoint_problems_are_reported() {
        let mut m = valid();
        m.presentation.entrypoint = "nope".into();
        m.entrypoints.get_mut("main").unwrap().path = "no-slash".into();
        assert_eq!(problems(&m).len(), 2);
        m.entrypoints.clear();
        assert!(problems(&m).iter().any(|p| p.contains("至少要有一个")));
    }

    #[test]
    fn paths_that_escape_the_package_are_rejected() {
        for bad in ["", "/abs", "../x", "a/../../b", "a\\b"] {
            let mut m = valid();
            m.runtime = Runtime::StaticWeb { source: bad.into() };
            assert_eq!(problems(&m).len(), 1, "source {bad:?}");
            let mut m = valid();
            m.presentation.icon = Some(bad.into());
            assert_eq!(problems(&m).len(), 1, "icon {bad:?}");
        }
        let mut m = valid();
        m.runtime = Runtime::StaticWeb { source: "a/b..c/d".into() };
        assert_eq!(m.validate(&HOST), Ok(()), "含 .. 的文件名不是 .. 段");
    }

    #[test]
    fn all_problems_are_collected_not_just_the_first() {
        let mut m = valid();
        m.name = " ".into();
        m.health = Health { path: "x".into(), timeout_ms: 0 };
        m.presentation.surface_hint = Some("Browser Panel".into());
        assert_eq!(problems(&m).len(), 4);
    }

    #[test]
    fn process_runtimes_need_a_command_and_a_valid_port_env() {
        let mut m = valid();
        m.runtime = Runtime::Python {
            command: vec![],
            lockfile: Some("../lock".into()),
            python: Some(">=3.12".into()),
            http: ProcessHttp { port_env: "port".into() },
        };
        assert_eq!(problems(&m).len(), 3);
        m.runtime = Runtime::Node {
            command: vec!["node".into(), "server.js".into()],
            lockfile: Some("package-lock.json".into()),
            node: None,
            http: ProcessHttp { port_env: "BYTEHOST_PORT".into() },
        };
        assert_eq!(m.validate(&HOST), Ok(()));
    }

    #[test]
    fn container_images_must_be_pinned_by_digest() {
        let digest = "a".repeat(64);
        let mut m = valid();
        for bad in ["docker.io/x/y:latest".to_string(), "docker.io/x/y@sha256:abc".to_string(), format!("@sha256:{digest}")] {
            m.runtime = Runtime::Container { image: bad.clone(), http: ContainerHttp { container_port: 3000 } };
            assert_eq!(problems(&m).len(), 1, "{bad}");
        }
        m.runtime = Runtime::Container {
            image: format!("docker.io/x/y@sha256:{digest}"),
            http: ContainerHttp { container_port: 3000 },
        };
        assert_eq!(m.validate(&HOST), Ok(()));
        m.runtime = Runtime::Container {
            image: format!("docker.io/x/y@sha256:{digest}"),
            http: ContainerHttp { container_port: 0 },
        };
        assert_eq!(problems(&m).len(), 1);
    }

    #[test]
    fn runtime_kind_names_match_the_manifest_vocabulary() {
        assert_eq!(valid().runtime.kind_name(), "static_web");
    }

    #[cfg(feature = "manifest-toml")]
    mod toml_text {
        use super::*;

        const EXCALIDRAW: &str = r#"
schema_version = 1
min_host_version = "0.1.0"
id = "excalidraw"
name = "Excalidraw"
version = "0.17.0"

[presentation]
icon = "assets/icon.svg"
surface_hint = "browser"
entrypoint = "main"

[entrypoints.main]
type = "web"
path = "/"
title = "Excalidraw"

[runtime]
kind = "static_web"
source = "web/"

[permissions]
clipboard = "read_write"
downloads = "user_confirm"
popups = "deny"

[permissions.network]
outbound = "none"

[permissions.filesystem]
data = "read_write"

[health]
path = "/"
timeout_ms = 3000
"#;

        #[test]
        fn the_spec_example_parses_to_the_expected_manifest() {
            let m = Manifest::from_toml(EXCALIDRAW, &HOST).unwrap();
            assert_eq!(m.id.as_str(), "excalidraw");
            assert_eq!(m.version, Version::new(0, 17, 0));
            assert_eq!(m.runtime, Runtime::StaticWeb { source: "web/".into() });
            assert_eq!(m.permissions.clipboard, crate::permissions::Access::ReadWrite);
            assert_eq!(m.permissions.filesystem.data, crate::permissions::Access::ReadWrite);
            assert_eq!(m.permissions.network.outbound, crate::permissions::Outbound::None);
            assert_eq!(m.entrypoints["main"].kind, EntrypointKind::Web);
        }

        #[test]
        fn unknown_fields_are_a_parse_error_at_the_top_level_and_in_nested_tables() {
            let top = format!("{EXCALIDRAW}\nsurprise = 1\n");
            assert!(matches!(Manifest::from_toml(&top, &HOST), Err(ManifestError::Parse(_))));
            let perms = EXCALIDRAW.replace("popups = \"deny\"", "popups = \"deny\"\ncamera = \"allow\"");
            assert!(matches!(Manifest::from_toml(&perms, &HOST), Err(ManifestError::Parse(_))));
            let runtime = EXCALIDRAW.replace("source = \"web/\"", "source = \"web/\"\nextra = true");
            assert!(matches!(Manifest::from_toml(&runtime, &HOST), Err(ManifestError::Parse(_))));
        }

        #[test]
        fn missing_required_fields_and_bad_values_are_parse_errors() {
            let no_id = EXCALIDRAW.replace("id = \"excalidraw\"\n", "");
            assert!(matches!(Manifest::from_toml(&no_id, &HOST), Err(ManifestError::Parse(_))));
            let bad_id = EXCALIDRAW.replace("id = \"excalidraw\"", "id = \"../etc\"");
            assert!(matches!(Manifest::from_toml(&bad_id, &HOST), Err(ManifestError::Parse(_))));
            let bad_gate = EXCALIDRAW.replace("user_confirm", "maybe");
            assert!(matches!(Manifest::from_toml(&bad_gate, &HOST), Err(ManifestError::Parse(_))));
            let bad_kind = EXCALIDRAW.replace("kind = \"static_web\"", "kind = \"wasm\"");
            assert!(matches!(Manifest::from_toml(&bad_kind, &HOST), Err(ManifestError::Parse(_))));
        }

        #[test]
        fn semantic_problems_surface_as_invalid_with_all_messages() {
            let text = EXCALIDRAW
                .replace("min_host_version = \"0.1.0\"", "min_host_version = \"9.0.0\"")
                .replace("source = \"web/\"", "source = \"../web\"");
            match Manifest::from_toml(&text, &HOST) {
                Err(ManifestError::Invalid(p)) => assert_eq!(p.len(), 2, "{p:?}"),
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn omitted_permissions_and_health_take_the_strictest_and_default_values() {
            let minimal = r#"
schema_version = 1
min_host_version = "0.1.0"
id = "tiny"
name = "Tiny"
version = "1.0.0"

[presentation]
entrypoint = "main"

[entrypoints.main]
type = "web"
path = "/"

[runtime]
kind = "static_web"
source = "web/"
"#;
            let m = Manifest::from_toml(minimal, &HOST).unwrap();
            assert_eq!(m.permissions, Permissions::default());
            assert_eq!(m.health, Health::default());
            assert_eq!(m.presentation.icon, None);
        }

        #[test]
        fn a_process_runtime_parses_from_toml() {
            let text = EXCALIDRAW.replace(
                "[runtime]\nkind = \"static_web\"\nsource = \"web/\"",
                "[runtime]\nkind = \"python\"\ncommand = [\"python\", \"-m\", \"app\"]\nlockfile = \"requirements.lock\"\n[runtime.http]\nport_env = \"BYTEHOST_PORT\"",
            );
            let m = Manifest::from_toml(&text, &HOST).unwrap();
            assert_eq!(m.runtime.kind_name(), "python");
        }
    }
}
```

`$SCRATCH/a0/tests_digest.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    #[test]
    fn sha256_matches_the_known_empty_and_abc_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn tree_digest_is_stable_and_independent_of_creation_order() {
        let a = tempfile::tempdir().unwrap();
        write(a.path(), "index.html", "<h1>x</h1>");
        write(a.path(), "js/app.js", "1");
        let b = tempfile::tempdir().unwrap();
        write(b.path(), "js/app.js", "1");
        write(b.path(), "index.html", "<h1>x</h1>");
        assert_eq!(digest_tree(a.path()).unwrap(), digest_tree(b.path()).unwrap());
        assert_eq!(digest_tree(a.path()).unwrap().len(), 64);
    }

    #[test]
    fn tree_digest_changes_with_content_name_addition_and_removal() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "index.html", "a");
        let base = digest_tree(d.path()).unwrap();

        write(d.path(), "index.html", "b");
        let edited = digest_tree(d.path()).unwrap();
        assert_ne!(edited, base, "内容变化");

        // 同样长度的新名字:否则"路径长度"会掩盖"路径内容没进摘要"
        fs::rename(d.path().join("index.html"), d.path().join("other.html")).unwrap();
        let renamed = digest_tree(d.path()).unwrap();
        assert_ne!(renamed, edited, "仅改名");

        write(d.path(), "extra.txt", "");
        let added = digest_tree(d.path()).unwrap();
        assert_ne!(added, renamed, "新增一个空文件");

        fs::remove_file(d.path().join("extra.txt")).unwrap();
        assert_eq!(digest_tree(d.path()).unwrap(), renamed, "删回去就回到原摘要");
    }

    #[test]
    fn tree_digest_cannot_be_confused_by_moving_bytes_between_name_and_content() {
        let a = tempfile::tempdir().unwrap();
        write(a.path(), "ab", "c");
        let b = tempfile::tempdir().unwrap();
        write(b.path(), "a", "bc");
        assert_ne!(digest_tree(a.path()).unwrap(), digest_tree(b.path()).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn tree_digest_refuses_symlinks_because_they_can_point_outside_the_package() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "index.html", "a");
        std::os::unix::fs::symlink("/etc/hosts", d.path().join("link")).unwrap();
        let err = digest_tree(d.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn data_store_id_is_deterministic_per_app_and_differs_between_apps() {
        let a = AppId::new("excalidraw").unwrap();
        let b = AppId::new("drawio").unwrap();
        assert_eq!(data_store_id(&a), data_store_id(&a));
        assert_ne!(data_store_id(&a), data_store_id(&b));
        assert_eq!(data_store_id_hex(&a).len(), 32);
        assert!(data_store_id_hex(&a).bytes().all(|c| c.is_ascii_hexdigit()));
    }
}
```

`$SCRATCH/a0/tests_plan.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        ContainerHttp, Entrypoint, EntrypointKind, Health, Presentation, ProcessHttp,
    };
    use crate::permissions::{Access, Gate};
    use std::collections::BTreeMap;

    fn manifest(runtime: Runtime, permissions: Permissions) -> Manifest {
        Manifest {
            schema_version: 1,
            min_host_version: Version::new(0, 1, 0),
            id: AppId::new("excalidraw").unwrap(),
            name: "Excalidraw".into(),
            version: Version::new(0, 17, 0),
            presentation: Presentation { icon: None, surface_hint: None, entrypoint: "main".into() },
            entrypoints: BTreeMap::from([(
                "main".to_string(),
                Entrypoint { kind: EntrypointKind::Web, path: "/".into(), title: None },
            )]),
            runtime,
            permissions,
            health: Health::default(),
        }
    }

    fn input<'a>(m: &'a Manifest, installed: Option<Installed<'a>>) -> PlanInput<'a> {
        PlanInput {
            manifest: m,
            manifest_digest: "m1".into(),
            source_digest: "s1".into(),
            provenance: Provenance::ThirdParty,
            trust: TrustLevel::Untrusted,
            enforcement: vec![EnforcementEntry {
                key: PermissionKey::NetworkOutbound,
                enforcement: Enforcement::Advisory,
            }],
            installed,
        }
    }

    fn approval() -> Approval {
        Approval { approver: "user".into(), approved_ms: 1 }
    }

    #[test]
    fn a_fresh_install_plan_carries_identity_provenance_digests_and_enforcement() {
        let perms = Permissions { clipboard: Access::ReadWrite, downloads: Gate::UserConfirm, ..Permissions::default() };
        let m = manifest(Runtime::StaticWeb { source: "web/".into() }, perms);
        let plan = InstallPlan::build(input(&m, None));
        assert_eq!(plan.app_id.as_str(), "excalidraw");
        assert_eq!(plan.upgrading_from, None);
        assert_eq!(plan.provenance, Provenance::ThirdParty);
        assert_eq!(plan.trust, TrustLevel::Untrusted);
        assert_eq!(plan.runtime_kind, "static_web");
        assert_eq!((plan.manifest_digest.as_str(), plan.source_digest.as_str()), ("m1", "s1"));
        assert_eq!(plan.requested, perms);
        assert_eq!(plan.enforcement[0].enforcement, Enforcement::Advisory, "强制等级原样带进计划");
        assert_eq!(plan.permission_diff.len(), 2);
        assert!(plan.permission_diff.iter().all(|c| c.escalation));
    }

    #[test]
    fn an_upgrade_plan_diffs_against_the_current_grants_not_against_nothing() {
        let granted = Permissions { clipboard: Access::ReadWrite, ..Permissions::default() };
        let requested = Permissions { clipboard: Access::ReadWrite, popups: Gate::UserConfirm, ..Permissions::default() };
        let m = manifest(Runtime::StaticWeb { source: "web/".into() }, requested);
        let v = Version::new(0, 16, 0);
        let plan = InstallPlan::build(input(&m, Some(Installed { version: &v, grants: &granted })));
        assert_eq!(plan.upgrading_from, Some(v));
        assert_eq!(plan.permission_diff.len(), 1, "剪贴板已授予,不再出现在差异里");
        assert_eq!(plan.permission_diff[0].key, PermissionKey::Popups);
    }

    #[test]
    fn will_run_describes_commands_honestly_for_each_runtime() {
        let stat = InstallPlan::build(input(&manifest(Runtime::StaticWeb { source: "web/".into() }, Permissions::default()), None));
        assert_eq!(stat.will_run.len(), 1);
        assert!(stat.will_run[0].contains("不执行任何命令"));

        let py = Runtime::Python {
            command: vec!["python".into(), "-m".into(), "app".into()],
            lockfile: Some("requirements.lock".into()),
            python: None,
            http: ProcessHttp { port_env: "PORT".into() },
        };
        let plan = InstallPlan::build(input(&manifest(py, Permissions::default()), None));
        assert_eq!(plan.will_run.len(), 2);
        assert!(plan.will_run[0].contains("requirements.lock") && plan.will_run[0].contains("任意代码"));
        assert_eq!(plan.will_run[1], "运行: python -m app");

        let digest = "b".repeat(64);
        let ct = Runtime::Container { image: format!("x/y@sha256:{digest}"), http: ContainerHttp { container_port: 80 } };
        let plan = InstallPlan::build(input(&manifest(ct, Permissions::default()), None));
        assert!(plan.will_run[0].contains(&digest));
    }

    #[test]
    fn an_approved_plan_verifies_only_against_the_same_digests() {
        let m = manifest(Runtime::StaticWeb { source: "web/".into() }, Permissions::default());
        let approved = InstallPlan::build(input(&m, None)).approve(approval());
        assert!(approved.verify("m1", "s1").is_ok());
        assert_eq!(approved.verify("m2", "s1"), Err(VerifyError::ManifestChanged));
        assert_eq!(approved.verify("m1", "s2"), Err(VerifyError::SourceChanged));
        assert_eq!(approved.verify("m2", "s2"), Err(VerifyError::ManifestChanged), "两者都变:先报 manifest");
    }

    #[test]
    fn an_approved_plan_exposes_the_plan_and_the_approval_record() {
        let m = manifest(Runtime::StaticWeb { source: "web/".into() }, Permissions::default());
        let approved = InstallPlan::build(input(&m, None)).approve(approval());
        assert_eq!(approved.plan().app_id.as_str(), "excalidraw");
        assert_eq!(approved.approval().approver, "user");
    }

    #[test]
    fn plans_round_trip_through_json_for_the_wire() {
        let m = manifest(Runtime::StaticWeb { source: "web/".into() }, Permissions::default());
        let approved = InstallPlan::build(input(&m, None)).approve(approval());
        let json = serde_json::to_string(&approved).unwrap();
        assert_eq!(serde_json::from_str::<ApprovedInstallPlan>(&json).unwrap(), approved);
    }
}
```

`$SCRATCH/a0/tests_state.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use DesiredState as D;
    use ObservedState as O;

    fn failed(retryable: bool) -> O {
        O::Failed { reason: "x".into(), retryable }
    }

    fn every_observed() -> Vec<O> {
        vec![
            O::NotInstalled,
            O::Installed,
            O::Preparing,
            O::Starting,
            O::Running,
            O::Stopping,
            O::Stopped,
            O::Updating,
            O::Uninstalling,
            failed(true),
            failed(false),
        ]
    }

    /// 完整真值表:3 种 desired × 11 种 observed,没有遗漏的格子。
    #[test]
    fn next_action_truth_table() {
        use Action::*;
        let table: [(D, Vec<Option<Action>>); 3] = [
            // NotInstalled, Installed, Preparing, Starting, Running, Stopping, Stopped, Updating, Uninstalling, Failed(retry), Failed(no)
            (D::Running, vec![None, Some(Start), None, None, None, None, Some(Start), None, None, Some(Start), None]),
            (D::Stopped, vec![None, None, Some(Stop), Some(Stop), Some(Stop), None, None, None, None, None, None]),
            (D::Removed, vec![None, Some(Uninstall), Some(Stop), Some(Stop), Some(Stop), None, Some(Uninstall), None, None, Some(Uninstall), Some(Uninstall)]),
        ];
        for (desired, expected) in table {
            for (observed, want) in every_observed().iter().zip(expected) {
                assert_eq!(next_action(desired, observed), want, "{desired:?} × {observed:?}");
            }
        }
    }

    #[test]
    fn transient_states_are_exactly_the_in_flight_ones() {
        let transient: Vec<_> = every_observed().into_iter().filter(|o| o.is_transient()).collect();
        assert_eq!(transient, vec![O::Preparing, O::Starting, O::Stopping, O::Updating, O::Uninstalling]);
    }

    #[test]
    fn a_non_retryable_failure_never_restarts_by_itself() {
        assert_eq!(next_action(D::Running, &failed(false)), None);
        assert_eq!(next_action(D::Running, &failed(true)), Some(Action::Start));
    }

    #[test]
    fn after_a_supervisor_restart_nothing_is_running_or_in_flight() {
        for (before, after) in [
            (O::Running, O::Stopped),
            (O::Starting, O::Stopped),
            (O::Preparing, O::Stopped),
            (O::Stopping, O::Stopped),
            (O::NotInstalled, O::NotInstalled),
            (O::Installed, O::Installed),
            (O::Stopped, O::Stopped),
        ] {
            assert_eq!(recover_after_supervisor_restart(before.clone()), after, "{before:?}");
        }
        assert_eq!(recover_after_supervisor_restart(failed(false)), failed(false), "失败原样保留");
        assert!(matches!(recover_after_supervisor_restart(O::Updating), O::Failed { retryable: true, .. }));
        assert!(matches!(recover_after_supervisor_restart(O::Uninstalling), O::Failed { retryable: true, .. }));
    }

    #[test]
    fn recovery_never_leaves_a_transient_state_behind() {
        for o in every_observed() {
            assert!(!recover_after_supervisor_restart(o.clone()).is_transient(), "{o:?}");
        }
    }

    #[test]
    fn a_desired_running_app_restarts_after_a_supervisor_restart() {
        let observed = recover_after_supervisor_restart(O::Running);
        assert_eq!(next_action(D::Running, &observed), Some(Action::Start));
    }

    #[test]
    fn states_serialize_with_a_stable_snake_case_vocabulary() {
        assert_eq!(serde_json::to_string(&D::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&O::NotInstalled).unwrap(), "{\"state\":\"not_installed\"}");
        let f = O::Failed { reason: "端口被占用".into(), retryable: true };
        assert_eq!(serde_json::from_str::<O>(&serde_json::to_string(&f).unwrap()).unwrap(), f);
    }
}
```

`$SCRATCH/a0/tests_event.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Version;
    use crate::manifest::{Entrypoint, EntrypointKind, Health, Presentation, Runtime};
    use crate::permissions::{Access, PermissionKey, Permissions};
    use std::collections::BTreeMap;

    fn manifest(permissions: Permissions) -> Manifest {
        Manifest {
            schema_version: 1,
            min_host_version: Version::new(0, 1, 0),
            id: AppId::new("excalidraw").unwrap(),
            name: "E".into(),
            version: Version::new(1, 0, 0),
            presentation: Presentation { icon: None, surface_hint: None, entrypoint: "main".into() },
            entrypoints: BTreeMap::from([(
                "main".to_string(),
                Entrypoint { kind: EntrypointKind::Web, path: "/".into(), title: None },
            )]),
            runtime: Runtime::StaticWeb { source: "web/".into() },
            permissions,
            health: Health::default(),
        }
    }

    fn id() -> AppId {
        AppId::new("excalidraw").unwrap()
    }

    #[test]
    fn every_event_reports_its_app() {
        let events = [
            AppEvent::Installed { app: id() },
            AppEvent::StateChanged { app: id(), state: ObservedState::Running },
            AppEvent::EndpointChanged { app: id(), url: Some("http://excalidraw.localhost:1/".into()) },
            AppEvent::ManifestChanged { app: id(), permission_changes: vec![] },
            AppEvent::Progress {
                app: id(),
                task: TaskId(1),
                progress: Progress { phase: "安装依赖".into(), done: 1, total: None },
            },
            AppEvent::LogAvailable { app: id() },
            AppEvent::RuntimeUnavailable {
                app: id(),
                reason: RuntimeReason::NotInstalled { runtime: "docker".into() },
            },
        ];
        for e in &events {
            assert_eq!(e.app(), &id());
        }
    }

    #[test]
    fn events_round_trip_through_json_with_tagged_names() {
        let e = AppEvent::RuntimeUnavailable {
            app: id(),
            reason: RuntimeReason::Unavailable { runtime: "docker".into(), detail: "Colima 没启动".into() },
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"event\":\"runtime_unavailable\""), "{json}");
        assert!(json.contains("\"reason\":\"unavailable\""), "{json}");
        assert_eq!(serde_json::from_str::<AppEvent>(&json).unwrap(), e);
    }

    #[test]
    fn manifest_changed_reports_only_real_permission_differences() {
        let old = manifest(Permissions { clipboard: Access::Read, ..Permissions::default() });
        let same = manifest(Permissions { clipboard: Access::Read, ..Permissions::default() });
        assert_eq!(manifest_changed(&old, &same), None);

        let more = manifest(Permissions { clipboard: Access::ReadWrite, ..Permissions::default() });
        match manifest_changed(&old, &more) {
            Some(AppEvent::ManifestChanged { app, permission_changes }) => {
                assert_eq!(app, id());
                assert_eq!(permission_changes.len(), 1);
                assert_eq!(permission_changes[0].key, PermissionKey::Clipboard);
                assert!(permission_changes[0].escalation);
            }
            other => panic!("{other:?}"),
        }
    }
}
```

`$SCRATCH/a0/tests_registry.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::Access;

    fn id(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    fn record(app: &str) -> AppRecord {
        AppRecord {
            id: id(app),
            desired: DesiredState::Running,
            current_version: Version::new(0, 17, 0),
            grants: Permissions { clipboard: Access::Read, ..Permissions::default() },
            versions: vec![VersionRecord {
                version: Version::new(0, 17, 0),
                manifest_digest: "m".into(),
                source_digest: "s".into(),
                installed_ms: 5,
            }],
            data_store_id: "0123456789abcdef0123456789abcdef".into(),
        }
    }

    #[test]
    fn paths_follow_the_documented_layout() {
        let p = AppPaths::new("/r");
        let a = id("excalidraw");
        assert_eq!(p.app_dir(&a), PathBuf::from("/r/apps/excalidraw"));
        assert_eq!(p.manifest_path(&a), PathBuf::from("/r/apps/excalidraw/manifest.toml"));
        assert_eq!(p.state_path(&a), PathBuf::from("/r/apps/excalidraw/state.json"));
        assert_eq!(p.package_dir(&a, &Version::new(0, 17, 0)), PathBuf::from("/r/apps/excalidraw/package/0.17.0"));
        assert_eq!(p.data_dir(&a), PathBuf::from("/r/apps/excalidraw/data"));
        assert_eq!(p.cache_dir(&a), PathBuf::from("/r/apps/excalidraw/cache"));
        assert_eq!(p.logs_dir(&a), PathBuf::from("/r/apps/excalidraw/logs"));
    }

    #[test]
    fn saving_creates_the_data_cache_and_logs_dirs_and_round_trips_the_record() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::open(tmp.path()).unwrap();
        let r = record("excalidraw");
        reg.save(&r).unwrap();
        for dir in [reg.paths().data_dir(&r.id), reg.paths().cache_dir(&r.id), reg.paths().logs_dir(&r.id)] {
            assert!(dir.is_dir(), "{}", dir.display());
        }
        assert_eq!(reg.load(&r.id).unwrap(), Some(r));
        assert_eq!(reg.load(&id("nope")).unwrap(), None);
    }

    #[test]
    fn saving_twice_replaces_the_record_and_leaves_no_temp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::open(tmp.path()).unwrap();
        let mut r = record("excalidraw");
        reg.save(&r).unwrap();
        r.desired = DesiredState::Stopped;
        reg.save(&r).unwrap();
        assert_eq!(reg.load(&r.id).unwrap().unwrap().desired, DesiredState::Stopped);
        let leftovers: Vec<_> = fs::read_dir(reg.paths().app_dir(&r.id))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn state_json_never_contains_secrets_only_what_the_record_defines() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::open(tmp.path()).unwrap();
        reg.save(&record("excalidraw")).unwrap();
        let text = fs::read_to_string(reg.paths().state_path(&id("excalidraw"))).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["current_version", "data_store_id", "desired", "grants", "id", "versions"]);
    }

    #[test]
    fn list_returns_apps_sorted_and_reports_corrupt_records_without_hiding_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::open(tmp.path()).unwrap();
        reg.save(&record("zeta")).unwrap();
        reg.save(&record("alpha")).unwrap();
        // 损坏的 state.json
        let bad = reg.paths().app_dir(&id("broken"));
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("state.json"), "{not json").unwrap();
        // 目录名与记录 id 不一致
        let liar = reg.paths().app_dir(&id("liar"));
        fs::create_dir_all(&liar).unwrap();
        fs::write(liar.join("state.json"), serde_json::to_string(&record("alpha")).unwrap()).unwrap();
        // 没有 state.json 的残留目录(如只保留了 data/)与普通文件:忽略
        fs::create_dir_all(reg.paths().app_dir(&id("leftover")).join("data")).unwrap();
        fs::write(reg.paths().apps_dir().join("stray.txt"), "x").unwrap();

        let listing = reg.list().unwrap();
        let ids: Vec<_> = listing.apps.iter().map(|a| a.id.as_str().to_string()).collect();
        assert_eq!(ids, ["alpha", "zeta"]);
        let mut problem_dirs: Vec<_> = listing.problems.iter().map(|(d, _)| d.as_str()).collect();
        problem_dirs.sort();
        assert_eq!(problem_dirs, ["broken", "liar"]);
    }

    #[test]
    fn load_rejects_a_record_whose_id_does_not_match_its_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::open(tmp.path()).unwrap();
        let liar = reg.paths().app_dir(&id("liar"));
        fs::create_dir_all(&liar).unwrap();
        fs::write(liar.join("state.json"), serde_json::to_string(&record("alpha")).unwrap()).unwrap();
        let err = reg.load(&id("liar")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    fn installed_with_everything(reg: &Registry, app: &str) -> AppId {
        let r = record(app);
        reg.save(&r).unwrap();
        reg.save_manifest(&r.id, "x = 1").unwrap();
        let pkg = reg.paths().package_dir(&r.id, &r.current_version);
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("index.html"), "hi").unwrap();
        fs::write(reg.paths().data_dir(&r.id).join("drawing.json"), "{}").unwrap();
        fs::write(reg.paths().cache_dir(&r.id).join("c"), "c").unwrap();
        fs::write(reg.paths().logs_dir(&r.id).join("l.log"), "l").unwrap();
        r.id
    }

    #[test]
    fn uninstalling_the_program_keeps_data_and_logs_and_a_reinstall_finds_them() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::open(tmp.path()).unwrap();
        let app = installed_with_everything(&reg, "excalidraw");
        reg.uninstall(&app, UninstallMode::Program).unwrap();

        assert_eq!(reg.load(&app).unwrap(), None);
        assert!(reg.list().unwrap().apps.is_empty(), "不再是已安装应用");
        assert!(!reg.paths().app_dir(&app).join("package").exists());
        assert!(!reg.paths().cache_dir(&app).exists());
        assert!(!reg.paths().manifest_path(&app).exists());
        assert_eq!(fs::read_to_string(reg.paths().data_dir(&app).join("drawing.json")).unwrap(), "{}");
        assert!(reg.paths().logs_dir(&app).join("l.log").exists());

        reg.save(&record("excalidraw")).unwrap();
        assert!(reg.paths().data_dir(&app).join("drawing.json").exists(), "重装后数据还在");
    }

    #[test]
    fn uninstalling_with_data_removes_the_whole_app_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::open(tmp.path()).unwrap();
        let app = installed_with_everything(&reg, "excalidraw");
        let other = installed_with_everything(&reg, "drawio");
        reg.uninstall(&app, UninstallMode::ProgramAndData).unwrap();
        assert!(!reg.paths().app_dir(&app).exists());
        assert!(reg.paths().data_dir(&other).join("drawing.json").exists(), "别的应用不受影响");
    }

    #[test]
    fn uninstalling_something_that_is_not_installed_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::open(tmp.path()).unwrap();
        for mode in [UninstallMode::Program, UninstallMode::ProgramAndData] {
            reg.uninstall(&id("ghost"), mode).unwrap();
        }
    }
}
```

`$SCRATCH/a0/Cargo.toml`:

```toml
[package]
name = "bytehost-apps"
version = "0.1.0"
edition.workspace = true
license.workspace = true
publish = false
description = "bytehost 应用宿主的无界面部分:应用模型、安装计划、生命周期状态、注册表。不依赖任何 dozer crate。"

[features]
# 默认 feature 为空:只有类型与纯逻辑,依赖仅 serde/serde_json——`dozer-core` 依赖本 crate 时,
# `dozer-hook`/`dozer-mcp` 的依赖闭包不会因此增加新 crate。
# 从 TOML 文本解析 manifest(dozerd 与安装流程用)。
manifest-toml = ["dep:toml"]
# 摘要计算(manifest 摘要、源码目录摘要、每应用数据存储标识)。
digest = ["dep:sha2"]

[dependencies]
serde.workspace = true
serde_json.workspace = true
toml = { version = "0.8", optional = true }
sha2 = { version = "0.10", optional = true }

[dev-dependencies]
tempfile = "3"
```

`$SCRATCH/a0/lib.rs`:

```rust
//! bytehost 应用宿主的无界面部分:应用模型、安装计划、生命周期状态、注册表。
//!
//! 设计见 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`。本 crate **不依赖任何 dozer crate**
//! (门禁:`scripts/check-bytehost-apps-deps.sh`),也不含界面/wry/iced。
//!
//! Cargo features(默认全空,只有类型与纯逻辑,依赖仅 serde/serde_json):
//! - `manifest-toml`:`Manifest::from_toml`
//! - `digest`:`digest` 模块(SHA-256、源码目录摘要、数据存储标识)

#[cfg(feature = "digest")]
pub mod digest;
pub mod event;
pub mod id;
pub mod manifest;
pub mod permissions;
pub mod plan;
pub mod registry;
pub mod state;
```

骨架脚本 `$SCRATCH/a0_skeleton.py`(带引号 heredoc),运行并确认编译失败:

```python
#!/usr/bin/env python3
"""A0 一次性脚本(步骤 3):建 crate 骨架——Cargo.toml、lib.rs、以及**只含测试**的各模块文件,
这样编译会因为缺类型而失败(RED)。`SP` 是 tests_*.rs / Cargo.toml / lib.rs 所在目录。**不提交。**"""
import os, shutil

SP = os.environ["SP"]
CRATE = "crates/bytehost-apps"
os.makedirs(CRATE + "/src", exist_ok=True)
shutil.copy(os.path.join(SP, "Cargo.toml"), CRATE + "/Cargo.toml")
shutil.copy(os.path.join(SP, "lib.rs"), CRATE + "/src/lib.rs")
for m in ("id", "permissions", "manifest", "digest", "plan", "state", "event", "registry"):
    shutil.copy(os.path.join(SP, f"tests_{m}.rs"), f"{CRATE}/src/{m}.rs")
print("ok")
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/a0 python3 $SCRATCH/a0_skeleton.py
cargo test -p bytehost-apps --all-features --no-run 2>&1 | grep -E "^error" | sort | uniq -c | head
```
Expected: 脚本输出 `ok`;编译失败(`could not compile bytehost-apps (lib test)`),报错是 `unresolved import`(如 `Action`、`crate::id::Version`、`crate::permissions::Access`)与 `cannot find …` 一类(RED——测试引用的类型和函数都还不存在)。

- [ ] **Step 4: 写实现源文件**

把下面 8 段实现写成 `$SCRATCH/a0/impl_<模块>.rs`,门禁脚本写成 `$SCRATCH/a0/check-bytehost-apps-deps.sh`:

`$SCRATCH/a0/impl_id.rs`:

```rust
//! 应用 id 与版本号:两个经过校验的小类型。

use std::fmt;

use serde::{Deserialize, Serialize};

/// 应用 id:稳定、小写、`[a-z0-9-]`,1–63 个字符,不以 `-` 开头或结尾。它同时是应用 origin 的主机名
/// (`<id>.localhost`)与磁盘目录名,所以字符集必须保守——这也让它天然不可能含路径分隔符。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AppId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    Empty,
    TooLong,
    BadChar(char),
    EdgeHyphen,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "应用 id 不能为空"),
            Self::TooLong => write!(f, "应用 id 最长 63 个字符"),
            Self::BadChar(c) => write!(f, "应用 id 只能含小写字母、数字和 '-',发现 {c:?}"),
            Self::EdgeHyphen => write!(f, "应用 id 不能以 '-' 开头或结尾"),
        }
    }
}

impl std::error::Error for IdError {}

impl AppId {
    pub fn new(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        if s.is_empty() {
            return Err(IdError::Empty);
        }
        if s.len() > 63 {
            return Err(IdError::TooLong);
        }
        if let Some(c) = s.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')) {
            return Err(IdError::BadChar(c));
        }
        if s.starts_with('-') || s.ends_with('-') {
            return Err(IdError::EdgeHyphen);
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AppId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, IdError> {
        Self::new(s)
    }
}

impl From<AppId> for String {
    fn from(id: AppId) -> String {
        id.0
    }
}

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `主.次.修订` 三段数字版本号(不支持预发布后缀:应用版本与宿主版本都只用这一种形式)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionError(pub String);

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "版本号必须是 主.次.修订 三段数字,收到 {:?}", self.0)
    }
}

impl std::error::Error for VersionError {}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    pub fn parse(s: &str) -> Result<Self, VersionError> {
        let bad = || VersionError(s.to_string());
        let mut it = s.split('.');
        let mut next = || -> Result<u32, VersionError> {
            let part = it.next().ok_or_else(bad)?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            part.parse().map_err(|_| bad())
        };
        let v = Self { major: next()?, minor: next()?, patch: next()? };
        if it.next().is_some() {
            return Err(bad());
        }
        Ok(v)
    }
}

impl TryFrom<String> for Version {
    type Error = VersionError;
    fn try_from(s: String) -> Result<Self, VersionError> {
        Self::parse(&s)
    }
}

impl From<Version> for String {
    fn from(v: Version) -> String {
        v.to_string()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
```

`$SCRATCH/a0/impl_permissions.rs`:

```rust
//! 权限模型:manifest 里的是**申请**(`Permissions`),用户**实际授予**的另存(`AppRecord.grants`,同一个类型)。
//! 每条权限还有强制等级(`Enforcement`),由 runtime adapter 在探测时给出——UI 必须原样展示。

use serde::{Deserialize, Serialize};

/// 读写访问级别,按从弱到强排序(`None < Read < ReadWrite`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    #[default]
    None,
    Read,
    ReadWrite,
}

/// 需要门控的能力,按从严到松排序(`Deny < UserConfirm < Allow`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    #[default]
    Deny,
    UserConfirm,
    Allow,
}

/// 出站网络(`None < Any`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outbound {
    #[default]
    None,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct NetworkPerm {
    pub outbound: Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FilesystemPerm {
    /// 应用自己的 `data/` 目录。
    pub data: Access,
}

/// 一组权限。缺省的字段取**最严**的值;未知字段整体拒绝(新版权限被旧宿主静默忽略是安全问题)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Permissions {
    pub network: NetworkPerm,
    pub filesystem: FilesystemPerm,
    pub clipboard: Access,
    pub downloads: Gate,
    pub popups: Gate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKey {
    NetworkOutbound,
    FilesystemData,
    Clipboard,
    Downloads,
    Popups,
}

impl PermissionKey {
    pub const ALL: [PermissionKey; 5] = [
        Self::NetworkOutbound,
        Self::FilesystemData,
        Self::Clipboard,
        Self::Downloads,
        Self::Popups,
    ];
}

/// 强制等级:该权限在这个 runtime 上能不能真正被执行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    /// 由宿主/runtime 真正强制(如静态 Web 的出站网络限制可由 CSP 强制)。
    Enforced,
    /// 只是声明,不被强制(如 macOS 上 Node/Python 的 `network: none`)。
    Advisory,
    /// 这个 runtime 完全不支持。
    Unsupported,
}

impl Permissions {
    /// 某条权限的(强度序号,展示用标签)。序号越大授予的能力越多。
    pub fn level(&self, key: PermissionKey) -> (u8, &'static str) {
        match key {
            PermissionKey::NetworkOutbound => match self.network.outbound {
                Outbound::None => (0, "none"),
                Outbound::Any => (1, "any"),
            },
            PermissionKey::FilesystemData => access(self.filesystem.data),
            PermissionKey::Clipboard => access(self.clipboard),
            PermissionKey::Downloads => gate(self.downloads),
            PermissionKey::Popups => gate(self.popups),
        }
    }
}

fn access(a: Access) -> (u8, &'static str) {
    match a {
        Access::None => (0, "none"),
        Access::Read => (1, "read"),
        Access::ReadWrite => (2, "read_write"),
    }
}

fn gate(g: Gate) -> (u8, &'static str) {
    match g {
        Gate::Deny => (0, "deny"),
        Gate::UserConfirm => (1, "user_confirm"),
        Gate::Allow => (2, "allow"),
    }
}

/// 申请与授予之间的一处差异。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionChange {
    pub key: PermissionKey,
    pub from: String,
    pub to: String,
    /// 申请比当前授予**更多**(需要用户重新确认);降低权限为 `false`。
    pub escalation: bool,
}

/// `requested`(manifest 申请)相对 `granted`(当前已授予)的全部差异,按 `PermissionKey::ALL` 的顺序。
/// 全新安装时 `granted` 传 `Permissions::default()`(什么都没授予)。
pub fn diff_permissions(granted: &Permissions, requested: &Permissions) -> Vec<PermissionChange> {
    PermissionKey::ALL
        .into_iter()
        .filter_map(|key| {
            let (from_n, from) = granted.level(key);
            let (to_n, to) = requested.level(key);
            (from_n != to_n).then(|| PermissionChange {
                key,
                from: from.to_string(),
                to: to.to_string(),
                escalation: to_n > from_n,
            })
        })
        .collect()
}
```

`$SCRATCH/a0/impl_manifest.rs`:

```rust
//! Manifest v1:应用对宿主的**申请**。字段、取值与校验规则见 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md` §4.1。
//! 所有结构 `deny_unknown_fields`;`validate` 一次性收集全部问题,不在第一个问题处停下。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::id::{AppId, Version};
use crate::permissions::Permissions;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    /// 能运行这个应用的最低宿主版本。
    pub min_host_version: Version,
    pub id: AppId,
    pub name: String,
    pub version: Version,
    pub presentation: Presentation,
    pub entrypoints: BTreeMap<String, Entrypoint>,
    pub runtime: Runtime,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub health: Health,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    /// 相对应用包根目录的图标路径。
    pub icon: Option<String>,
    /// 中性的承载提示(如 `browser`),宿主产品可以忽略;不使用 `browser_panel` 这类产品词。
    pub surface_hint: Option<String>,
    /// 默认入口,必须是 `entrypoints` 里的键。
    pub entrypoint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entrypoint {
    #[serde(rename = "type")]
    pub kind: EntrypointKind,
    /// 应用内路径,以 `/` 开头。
    pub path: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrypointKind {
    Web,
}

/// 进程型应用怎么对外提供 HTTP:宿主把端口放进这个环境变量传给应用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessHttp {
    pub port_env: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerHttp {
    pub container_port: u16,
}

/// 运行方式。一期只实现 `StaticWeb`;其余只定义形状,供 adapter 的 `probe` 与安装计划使用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Runtime {
    StaticWeb {
        /// 相对应用包根目录的静态文件目录。
        source: String,
    },
    Node {
        command: Vec<String>,
        lockfile: Option<String>,
        node: Option<String>,
        http: ProcessHttp,
    },
    Python {
        command: Vec<String>,
        lockfile: Option<String>,
        python: Option<String>,
        http: ProcessHttp,
    },
    Container {
        /// 必须按摘要固定:`name@sha256:<64 位十六进制>`。
        image: String,
        http: ContainerHttp,
    },
}

impl Runtime {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::StaticWeb { .. } => "static_web",
            Self::Node { .. } => "node",
            Self::Python { .. } => "python",
            Self::Container { .. } => "container",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Health {
    pub path: String,
    pub timeout_ms: u64,
}

impl Default for Health {
    fn default() -> Self {
        Self { path: "/".to_string(), timeout_ms: 3000 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// TOML 语法错误、缺字段、未知字段、取值不在枚举内。
    Parse(String),
    /// 语法合法,但有一个或多个语义问题。
    Invalid(Vec<String>),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "manifest 解析失败: {e}"),
            Self::Invalid(problems) => write!(f, "manifest 不合法: {}", problems.join("; ")),
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    /// 解析 TOML 文本并校验。`host_version` 用来检查 `min_host_version`。
    #[cfg(feature = "manifest-toml")]
    pub fn from_toml(text: &str, host_version: &Version) -> Result<Self, ManifestError> {
        let manifest: Manifest = toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))?;
        manifest.validate(host_version).map_err(ManifestError::Invalid)?;
        Ok(manifest)
    }

    /// 语义校验,返回全部问题。
    pub fn validate(&self, host_version: &Version) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.schema_version != SCHEMA_VERSION {
            problems.push(format!("schema_version 必须是 {SCHEMA_VERSION},收到 {}", self.schema_version));
        }
        if self.min_host_version > *host_version {
            problems.push(format!(
                "需要宿主版本 >= {},当前宿主是 {host_version}",
                self.min_host_version
            ));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 64 {
            problems.push("name 不能为空且最长 64 个字符".to_string());
        }
        if self.entrypoints.is_empty() {
            problems.push("entrypoints 至少要有一个".to_string());
        }
        if !self.entrypoints.contains_key(&self.presentation.entrypoint) {
            problems.push(format!(
                "presentation.entrypoint {:?} 不在 entrypoints 里",
                self.presentation.entrypoint
            ));
        }
        for (name, ep) in &self.entrypoints {
            if !ep.path.starts_with('/') {
                problems.push(format!("entrypoints.{name}.path 必须以 '/' 开头"));
            }
        }
        if let Some(icon) = &self.presentation.icon {
            check_relative("presentation.icon", icon, &mut problems);
        }
        if let Some(hint) = &self.presentation.surface_hint
            && (hint.is_empty() || hint.len() > 32 || !hint.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        {
            problems.push("presentation.surface_hint 只能是 1–32 个小写字母或 '_'".to_string());
        }
        self.validate_runtime(&mut problems);
        if !self.health.path.starts_with('/') {
            problems.push("health.path 必须以 '/' 开头".to_string());
        }
        if !(1..=60_000).contains(&self.health.timeout_ms) {
            problems.push("health.timeout_ms 必须在 1..=60000".to_string());
        }
        if problems.is_empty() { Ok(()) } else { Err(problems) }
    }

    fn validate_runtime(&self, problems: &mut Vec<String>) {
        match &self.runtime {
            Runtime::StaticWeb { source } => check_relative("runtime.source", source, problems),
            Runtime::Node { command, lockfile, http, .. } | Runtime::Python { command, lockfile, http, .. } => {
                if command.is_empty() || command[0].trim().is_empty() {
                    problems.push("runtime.command 不能为空".to_string());
                }
                if let Some(lock) = lockfile {
                    check_relative("runtime.lockfile", lock, problems);
                }
                if !is_env_name(&http.port_env) {
                    problems.push("runtime.http.port_env 必须是 [A-Z_][A-Z0-9_]* 形式的环境变量名".to_string());
                }
            }
            Runtime::Container { image, http } => {
                if !is_pinned_image(image) {
                    problems.push("runtime.image 必须按摘要固定,形如 name@sha256:<64 位十六进制>".to_string());
                }
                if http.container_port == 0 {
                    problems.push("runtime.http.container_port 不能为 0".to_string());
                }
            }
        }
    }
}

/// 相对路径:非空、不以 `/` 开头、不含 `..` 段、不含反斜杠或 NUL。
fn check_relative(field: &str, path: &str, problems: &mut Vec<String>) {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.split('/').any(|seg| seg == "..");
    if bad {
        problems.push(format!("{field} 必须是应用包内的相对路径(不能为空、不能绝对、不能含 ..),收到 {path:?}"));
    }
}

fn is_env_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_uppercase() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

fn is_pinned_image(image: &str) -> bool {
    match image.split_once("@sha256:") {
        Some((name, digest)) => {
            !name.is_empty() && digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
        }
        None => false,
    }
}
```

`$SCRATCH/a0/impl_digest.rs`:

```rust
//! 摘要(feature `digest`):manifest 摘要、源码目录摘要、每应用数据存储标识。
//! 审批绑定摘要——审批之后源码或 manifest 被替换,安装时摘要对不上就拒绝(防 TOCTOU)。

use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::id::AppId;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 字节串的 SHA-256,小写十六进制(64 个字符)。
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// 目录树摘要:按相对路径(`/` 分隔)字典序遍历所有文件,逐个把"路径长度、路径、内容长度、内容"喂给
/// SHA-256。文件内容、文件名、文件增删、目录结构变化都会改变摘要;遇到符号链接直接报错
/// (`InvalidInput`)——链接可以指向包外,摘要无法代表它的内容。空目录不计入(只有文件)。
pub fn digest_tree(root: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for rel in &files {
        let content = fs::read(root.join(rel))?;
        hasher.update((rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update((content.len() as u64).to_le_bytes());
        hasher.update(&content);
    }
    Ok(hex(&hasher.finalize()))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("应用包里不允许符号链接: {}", path.display()),
            ));
        }
        if file_type.is_dir() {
            collect(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).expect("在 root 之下");
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(rel);
        }
    }
    Ok(())
}

/// 每应用一个稳定的 WebView 数据存储标识(16 字节):由 app id 确定性派生,卸载程序再重装后不变
/// (因此保留下来的 `data/` 与浏览器存储仍然对得上)。不同 id 得到不同标识,实测不同标识即使 origin
/// 相同,localStorage/IndexedDB/Cookie 也互相隔离。
pub fn data_store_id(id: &AppId) -> [u8; 16] {
    let digest = Sha256::digest(format!("bytehost-app-data-store:{id}").as_bytes());
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

/// 同 [`data_store_id`],小写十六进制(32 个字符),落盘用。
pub fn data_store_id_hex(id: &AppId) -> String {
    hex(&data_store_id(id))
}
```

`$SCRATCH/a0/impl_plan.rs`:

```rust
//! 安装计划与审批:`install_plan → 审批 → install`。bytehost 只产出**可审查的信息**并保证"审批的就是装的",
//! **是否需要用户确认由产品决定**(Dozer 可以要求,Digger 可以更宽松)。

use serde::{Deserialize, Serialize};

use crate::id::{AppId, Version};
use crate::manifest::{Manifest, Runtime};
use crate::permissions::{Enforcement, PermissionChange, PermissionKey, Permissions, diff_permissions};

/// 应用从哪来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// 用户本地目录/文件。
    Local,
    /// Agent 现场生成。
    AgentGenerated,
    /// 第三方来源(下载、仓库)。
    ThirdParty,
}

/// 信任级别:由**产品**根据来源与自己的策略给出,bytehost 只记录并展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Trusted,
    Untrusted,
}

/// 一条权限在目标 runtime 上的强制等级(由 runtime adapter 的 `probe` 给出)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnforcementEntry {
    pub key: PermissionKey,
    pub enforcement: Enforcement,
}

/// 安装计划:UI 需要给用户看的全部信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub app_id: AppId,
    pub name: String,
    pub version: Version,
    /// 升级时是当前已安装版本,全新安装为 `None`。
    pub upgrading_from: Option<Version>,
    pub provenance: Provenance,
    pub trust: TrustLevel,
    pub runtime_kind: String,
    pub manifest_digest: String,
    pub source_digest: String,
    /// manifest 申请的权限。
    pub requested: Permissions,
    pub enforcement: Vec<EnforcementEntry>,
    /// 申请相对"当前已授予"的差异(全新安装时相对"什么都没授予")。
    pub permission_diff: Vec<PermissionChange>,
    /// 安装/运行时会执行什么——人类可读的描述,UI 原样展示。
    pub will_run: Vec<String>,
}

/// 当前已安装的版本与已授予的权限(升级时用来算差异)。
#[derive(Debug, Clone, Copy)]
pub struct Installed<'a> {
    pub version: &'a Version,
    pub grants: &'a Permissions,
}

/// 构造安装计划所需的全部输入(具名字段,避免相邻的同类型参数传错)。
pub struct PlanInput<'a> {
    pub manifest: &'a Manifest,
    pub manifest_digest: String,
    pub source_digest: String,
    pub provenance: Provenance,
    pub trust: TrustLevel,
    pub enforcement: Vec<EnforcementEntry>,
    pub installed: Option<Installed<'a>>,
}

impl InstallPlan {
    pub fn build(input: PlanInput<'_>) -> Self {
        let m = input.manifest;
        let baseline = input.installed.map(|i| *i.grants).unwrap_or_default();
        Self {
            app_id: m.id.clone(),
            name: m.name.clone(),
            version: m.version,
            upgrading_from: input.installed.map(|i| *i.version),
            provenance: input.provenance,
            trust: input.trust,
            runtime_kind: m.runtime.kind_name().to_string(),
            manifest_digest: input.manifest_digest,
            source_digest: input.source_digest,
            requested: m.permissions,
            enforcement: input.enforcement,
            permission_diff: diff_permissions(&baseline, &m.permissions),
            will_run: will_run(&m.runtime),
        }
    }

    /// 产品/用户批准这份计划。批准的内容就是这份计划(含两个摘要)。
    pub fn approve(self, approval: Approval) -> ApprovedInstallPlan {
        ApprovedInstallPlan { plan: self, approval }
    }
}

fn will_run(runtime: &Runtime) -> Vec<String> {
    match runtime {
        Runtime::StaticWeb { source } => vec![format!("不执行任何命令;由宿主直接提供 {source} 里的静态文件")],
        Runtime::Node { command, lockfile, .. } | Runtime::Python { command, lockfile, .. } => {
            let mut out = Vec::new();
            if let Some(lock) = lockfile {
                out.push(format!("按 {lock} 安装依赖(依赖的安装脚本本身可以执行任意代码)"));
            }
            out.push(format!("运行: {}", command.join(" ")));
            out
        }
        Runtime::Container { image, .. } => vec![format!("拉取镜像 {image}"), "以容器方式运行".to_string()],
    }
}

/// 谁在什么时候批准的(审计用)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub approver: String,
    pub approved_ms: u64,
}

/// 已批准的计划。字段私有、只能经 [`InstallPlan::approve`] 构造,安装前必须 [`verify`](Self::verify)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovedInstallPlan {
    plan: InstallPlan,
    approval: Approval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    ManifestChanged,
    SourceChanged,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ManifestChanged => write!(f, "manifest 在审批之后发生了变化"),
            Self::SourceChanged => write!(f, "应用源码在审批之后发生了变化"),
        }
    }
}

impl std::error::Error for VerifyError {}

impl ApprovedInstallPlan {
    pub fn plan(&self) -> &InstallPlan {
        &self.plan
    }

    pub fn approval(&self) -> &Approval {
        &self.approval
    }

    /// 安装时用**此刻重新计算**的两个摘要核对:任何一个与审批时不同就拒绝。
    pub fn verify(&self, manifest_digest: &str, source_digest: &str) -> Result<&InstallPlan, VerifyError> {
        if self.plan.manifest_digest != manifest_digest {
            return Err(VerifyError::ManifestChanged);
        }
        if self.plan.source_digest != source_digest {
            return Err(VerifyError::SourceChanged);
        }
        Ok(&self.plan)
    }
}
```

`$SCRATCH/a0/impl_state.rs`:

```rust
//! 生命周期:`desired`(想要什么)与 `observed`(实际观察到什么)分开,由 manager 持续对账。
//! 这里只有**纯函数**:给定两者,下一步该做什么;宿主(supervisor)重启之后,持久化的观察态怎么修正。

use serde::{Deserialize, Serialize};

/// 用户/产品想要应用处于什么状态(持久化在 `AppRecord`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    Stopped,
    Running,
    /// 要卸载(程序;是否连数据一起删由卸载模式决定)。
    Removed,
}

/// 实际观察到的状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ObservedState {
    NotInstalled,
    Installed,
    Preparing,
    Starting,
    Running,
    Stopping,
    Stopped,
    Updating,
    Uninstalling,
    Failed { reason: String, retryable: bool },
}

/// 对账给出的下一步动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 准备(装依赖/拉镜像等)并启动。
    Start,
    /// 停止(含取消进行中的准备/启动)。
    Stop,
    Uninstall,
}

impl ObservedState {
    /// 进行中的过渡态:动作已经发出,对账时应当等它结束而不是再发一次。
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Starting | Self::Stopping | Self::Updating | Self::Uninstalling
        )
    }
}

/// 想要 `desired` 而实际是 `observed` 时,下一步该做什么;`None` = 什么都不用做(已达成、在等进行中的动作、
/// 或不可恢复的失败需要人介入)。
pub fn next_action(desired: DesiredState, observed: &ObservedState) -> Option<Action> {
    use DesiredState as D;
    use ObservedState as O;
    match (desired, observed) {
        // 进行中的过渡态:等,但"想停/想卸"时仍要能取消正在准备/启动的
        (D::Stopped | D::Removed, O::Preparing | O::Starting) => Some(Action::Stop),
        (_, o) if o.is_transient() => None,

        (D::Running, O::Installed | O::Stopped) => Some(Action::Start),
        (D::Running, O::Failed { retryable: true, .. }) => Some(Action::Start),
        (D::Running, _) => None,

        (D::Stopped, O::Running) => Some(Action::Stop),
        (D::Stopped, _) => None,

        (D::Removed, O::Running) => Some(Action::Stop),
        (D::Removed, O::Installed | O::Stopped | O::Failed { .. }) => Some(Action::Uninstall),
        (D::Removed, _) => None,
    }
}

/// supervisor(dozerd)重启之后,把持久化下来的观察态修正为现实:应用随 supervisor 一起停止了
/// (`docs/.../app-host-design.md` §6.1),所以"曾在运行/过渡中"的都回到 `Stopped`;
/// 被打断的升级/卸载无法判断包是否完整,标成可重试的失败。
pub fn recover_after_supervisor_restart(observed: ObservedState) -> ObservedState {
    match observed {
        ObservedState::Preparing | ObservedState::Starting | ObservedState::Running | ObservedState::Stopping => {
            ObservedState::Stopped
        }
        ObservedState::Updating => ObservedState::Failed { reason: "升级被打断".to_string(), retryable: true },
        ObservedState::Uninstalling => ObservedState::Failed { reason: "卸载被打断".to_string(), retryable: true },
        other => other,
    }
}
```

`$SCRATCH/a0/impl_event.rs`:

```rust
//! 给产品层的事件:Rail 据此更新图标状态;bytehost 不理解"左栏/右栏"。

use serde::{Deserialize, Serialize};

use crate::id::AppId;
use crate::manifest::Manifest;
use crate::permissions::PermissionChange;
use crate::state::ObservedState;

/// 一个耗时任务(安装/准备/启动…)的标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// 当前阶段的人类可读名称(如 "安装依赖"、"拉取镜像")。
    pub phase: String,
    pub done: u64,
    /// 总量未知时为 `None`。
    pub total: Option<u64>,
}

/// 运行时不可用的原因(产品据此在应用面板里画提示页,见规格 §6.3)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum RuntimeReason {
    /// 系统里没有这个运行时(如没装 docker/node/uv)。
    NotInstalled { runtime: String },
    /// 装了但当前用不了(如 Colima 没启动)。
    Unavailable { runtime: String, detail: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AppEvent {
    Installed { app: AppId },
    StateChanged { app: AppId, state: ObservedState },
    /// 应用的访问地址变化(启动后出现、停止后消失)。
    EndpointChanged { app: AppId, url: Option<String> },
    /// 升级后 manifest 的权限变化(供产品展示)。
    ManifestChanged { app: AppId, permission_changes: Vec<PermissionChange> },
    Progress { app: AppId, task: TaskId, progress: Progress },
    LogAvailable { app: AppId },
    RuntimeUnavailable { app: AppId, reason: RuntimeReason },
}

impl AppEvent {
    /// 这条事件属于哪个应用。
    pub fn app(&self) -> &AppId {
        match self {
            Self::Installed { app }
            | Self::StateChanged { app, .. }
            | Self::EndpointChanged { app, .. }
            | Self::ManifestChanged { app, .. }
            | Self::Progress { app, .. }
            | Self::LogAvailable { app }
            | Self::RuntimeUnavailable { app, .. } => app,
        }
    }
}

/// 比较新旧 manifest 的权限申请,给出 `ManifestChanged` 需要的差异;没有变化返回 `None`。
pub fn manifest_changed(old: &Manifest, new: &Manifest) -> Option<AppEvent> {
    let changes = crate::permissions::diff_permissions(&old.permissions, &new.permissions);
    (!changes.is_empty()).then(|| AppEvent::ManifestChanged { app: new.id.clone(), permission_changes: changes })
}
```

`$SCRATCH/a0/impl_registry.rs`:

```rust
//! 注册表与磁盘布局:`<root>/apps/<app-id>/{manifest.toml, state.json, package/<ver>/, data/, cache/, logs/}`。
//! 程序包、用户数据、缓存、日志分开,卸载"程序"与删除"数据"是两个操作。纯同步文件 I/O,无异步、无网络。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::id::{AppId, Version};
use crate::permissions::Permissions;
use crate::state::DesiredState;

/// 一个应用在磁盘上的各个位置。
#[derive(Debug, Clone)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    /// `root` 是宿主为 bytehost 分配的数据目录。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn apps_dir(&self) -> PathBuf {
        self.root.join("apps")
    }

    pub fn app_dir(&self, id: &AppId) -> PathBuf {
        self.apps_dir().join(id.as_str())
    }

    pub fn manifest_path(&self, id: &AppId) -> PathBuf {
        self.app_dir(id).join("manifest.toml")
    }

    pub fn state_path(&self, id: &AppId) -> PathBuf {
        self.app_dir(id).join("state.json")
    }

    /// 某个版本的不可变程序内容目录。
    pub fn package_dir(&self, id: &AppId, version: &Version) -> PathBuf {
        self.app_dir(id).join("package").join(version.to_string())
    }

    pub fn data_dir(&self, id: &AppId) -> PathBuf {
        self.app_dir(id).join("data")
    }

    pub fn cache_dir(&self, id: &AppId) -> PathBuf {
        self.app_dir(id).join("cache")
    }

    pub fn logs_dir(&self, id: &AppId) -> PathBuf {
        self.app_dir(id).join("logs")
    }
}

/// 一次安装留下的版本记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRecord {
    pub version: Version,
    pub manifest_digest: String,
    pub source_digest: String,
    pub installed_ms: u64,
}

/// 持久化的应用记录(`state.json`)。**不含密钥**——密钥只存引用,由宿主的凭据服务注入。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppRecord {
    pub id: AppId,
    pub desired: DesiredState,
    /// 当前生效的版本(`versions` 里最后一次安装的那个)。
    pub current_version: Version,
    /// 用户**实际授予**的权限(与 manifest 的"申请"分开存)。
    pub grants: Permissions,
    pub versions: Vec<VersionRecord>,
    /// 每应用 WebView 数据存储标识(32 位十六进制),跨重启、跨卸载重装稳定。
    pub data_store_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninstallMode {
    /// 只删程序:保留 `data/` 与 `logs/`,重装后数据还在。
    Program,
    /// 连数据一起删:整个应用目录。
    ProgramAndData,
}

/// `list` 的结果:读得出来的记录,加上读不出来的(损坏的 `state.json` 不应遮住别的应用)。
#[derive(Debug, Default)]
pub struct Listing {
    pub apps: Vec<AppRecord>,
    /// (目录名, 问题描述)。
    pub problems: Vec<(String, String)>,
}

pub struct Registry {
    paths: AppPaths,
}

impl Registry {
    /// 打开(必要时创建)`<root>/apps/`。
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let paths = AppPaths::new(root);
        fs::create_dir_all(paths.apps_dir())?;
        Ok(Self { paths })
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// 全部已登记应用,按 id 排序。没有 `state.json` 的目录(如只保留了数据的残留目录)不算已安装应用。
    pub fn list(&self) -> io::Result<Listing> {
        let mut listing = Listing::default();
        let mut dirs: Vec<_> = fs::read_dir(self.paths.apps_dir())?.collect::<io::Result<_>>()?;
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let state = entry.path().join("state.json");
            if !state.exists() {
                continue;
            }
            match read_record(&state, &name) {
                Ok(record) => listing.apps.push(record),
                Err(problem) => listing.problems.push((name, problem)),
            }
        }
        Ok(listing)
    }

    pub fn load(&self, id: &AppId) -> io::Result<Option<AppRecord>> {
        let path = self.paths.state_path(id);
        match fs::read_to_string(&path) {
            Ok(text) => {
                let record: AppRecord = serde_json::from_str(&text)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?;
                if &record.id != id {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{}: 记录里的 id {} 与目录名不一致", path.display(), record.id),
                    ));
                }
                Ok(Some(record))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 原子地写 `state.json`(先写临时文件再改名),并确保 `data/`、`cache/`、`logs/` 存在。
    pub fn save(&self, record: &AppRecord) -> io::Result<()> {
        let id = &record.id;
        for dir in [self.paths.data_dir(id), self.paths.cache_dir(id), self.paths.logs_dir(id)] {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
        write_atomic(&self.paths.state_path(id), json.as_bytes())
    }

    /// 原子地写 `manifest.toml`(当前版本的申请原文)。
    pub fn save_manifest(&self, id: &AppId, text: &str) -> io::Result<()> {
        fs::create_dir_all(self.paths.app_dir(id))?;
        write_atomic(&self.paths.manifest_path(id), text.as_bytes())
    }

    /// 卸载。应用不存在不算错误(幂等)。
    pub fn uninstall(&self, id: &AppId, mode: UninstallMode) -> io::Result<()> {
        let dir = self.paths.app_dir(id);
        match mode {
            UninstallMode::ProgramAndData => remove_dir_if_exists(&dir),
            UninstallMode::Program => {
                remove_dir_if_exists(&dir.join("package"))?;
                remove_dir_if_exists(&dir.join("cache"))?;
                remove_file_if_exists(&self.paths.manifest_path(id))?;
                // state.json 最后删:它一删,应用就不再出现在 `list` 里
                remove_file_if_exists(&self.paths.state_path(id))
            }
        }
    }
}

fn read_record(path: &Path, dir_name: &str) -> Result<AppRecord, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let record: AppRecord = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if record.id.as_str() != dir_name {
        return Err(format!("记录里的 id {} 与目录名 {dir_name} 不一致", record.id));
    }
    Ok(record)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn remove_dir_if_exists(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn remove_file_if_exists(file: &Path) -> io::Result<()> {
    match fs::remove_file(file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
```

`$SCRATCH/a0/check-bytehost-apps-deps.sh`:

```bash
#!/usr/bin/env bash
# 门禁:bytehost-apps 是要让 Digger 也能用的、与产品无关的 crate。
#  1. 任何 feature 组合下,依赖树里都不得出现 dozer* crate(不依赖 Dozer 的任何东西);
#  2. 默认 feature(只有类型与纯逻辑)的依赖闭包只能是 serde/serde_json 家族——`dozer-core` 依赖它时,
#     `dozer-hook`/`dozer-mcp` 才不会因此多出新 crate。
set -euo pipefail
cd "$(dirname "$0")/.."

all=$(cargo tree -p bytehost-apps -e normal --all-features --prefix none 2>/dev/null | awk '{print $1}' | sort -u)
if echo "$all" | grep -qE '^dozer'; then
  echo "bytehost-apps 不得依赖 dozer* crate,发现:" >&2
  echo "$all" | grep -E '^dozer' >&2
  exit 1
fi

allowed='^(bytehost-apps|serde|serde_core|serde_derive|serde_json|proc-macro2|quote|syn|unicode-ident|itoa|memchr|zmij)$'
extra=$(cargo tree -p bytehost-apps -e normal --prefix none 2>/dev/null | awk '{print $1}' | sort -u | grep -Ev "$allowed" || true)
if [ -n "$extra" ]; then
  echo "bytehost-apps 默认 feature 的依赖闭包里出现了不在白名单里的 crate(新增依赖请放进 feature):" >&2
  echo "$extra" >&2
  exit 1
fi
echo "bytehost-apps deps check: ok"
```

- [ ] **Step 5: 装配并跑测试**

装配脚本 `$SCRATCH/a0_assemble.py`(带引号 heredoc):

```python
#!/usr/bin/env python3
"""A0 一次性脚本(步骤 5):把各模块的实现写到测试前面,并装上依赖门禁脚本。`SP` 同上。**不提交。**"""
import os, shutil, stat

SP = os.environ["SP"]
CRATE = "crates/bytehost-apps"
for m in ("id", "permissions", "manifest", "digest", "plan", "state", "event", "registry"):
    impl = open(os.path.join(SP, f"impl_{m}.rs")).read()
    tests = open(os.path.join(SP, f"tests_{m}.rs")).read()
    open(f"{CRATE}/src/{m}.rs", "w").write(impl + tests)
shutil.copy(os.path.join(SP, "check-bytehost-apps-deps.sh"), "scripts/check-bytehost-apps-deps.sh")
os.chmod("scripts/check-bytehost-apps-deps.sh", os.stat("scripts/check-bytehost-apps-deps.sh").st_mode | stat.S_IXUSR)
print("ok")
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0 && export PYTHONDONTWRITEBYTECODE=1
SP=$SCRATCH/a0 python3 $SCRATCH/a0_assemble.py && cargo fmt -p bytehost-apps && git status --short
cargo test -p bytehost-apps --all-features 2>&1 | grep -E "^error|FAILED|test result"
```
Expected: 装配脚本输出 `ok`;`git status --short` 是 `M Cargo.lock`、`?? crates/bytehost-apps/`、`?? scripts/check-bytehost-apps-deps.sh`;`test result: ok. 59 passed; 0 failed`。若有失败,用 systematic-debugging 找原因——**不要改测试迁就实现**。

- [ ] **Step 6: 每个 feature 组合、clippy、依赖门禁、锁文件**

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0
cargo test -p bytehost-apps 2>&1 | grep -E "^error|test result: .* [1-9][0-9]* passed"
cargo test -p bytehost-apps --features digest 2>&1 | grep -E "^error|test result: .* [1-9][0-9]* passed"
cargo test -p bytehost-apps --features manifest-toml 2>&1 | grep -E "^error|test result: .* [1-9][0-9]* passed"
cargo clippy -p bytehost-apps --all-targets --all-features 2>&1 | grep -E "^(warning|error)" | head -3; echo "clippy(all) done"
cargo clippy -p bytehost-apps --all-targets 2>&1 | grep -E "^(warning|error)" | head -3; echo "clippy(default) done"
./scripts/check-bytehost-apps-deps.sh
git diff Cargo.lock | grep '^[+-]' | grep -v '^+++\|^---'
```
Expected: 默认 **47** passed、`digest` **53**、`manifest-toml` **53**;两次 clippy 都**没有**任何 `warning`/`error` 行(只有 `done` 回显);门禁输出 `bytehost-apps deps check: ok`;最后一条的输出恰好是 `bytehost-apps` 一个 `[[package]]` 块的 11 行新增(`+[[package]]`、`+name = "bytehost-apps"`、`+version = "0.1.0"`、`+dependencies = [` 及其 5 项、`+]`、一个空 `+` 行),**没有任何 `-` 行,没有 `source =` 行**。

- [ ] **Step 7: 门禁自身的变异(证明它真的会拦)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0
cp crates/bytehost-apps/Cargo.toml $SCRATCH/Cargo.toml.bak
# (a) 把 sha2 从可选依赖改成默认依赖
sed -i '' 's/^sha2 = { version = "0.10", optional = true }/sha2 = { version = "0.10" }/; s/^digest = \["dep:sha2"\]/digest = []/' crates/bytehost-apps/Cargo.toml
./scripts/check-bytehost-apps-deps.sh; echo "exit=$?"; cp $SCRATCH/Cargo.toml.bak crates/bytehost-apps/Cargo.toml
# (b) 让某个 feature 依赖 dozer-core
sed -i '' 's/^serde_json.workspace = true/serde_json.workspace = true\ndozer-core = { path = "..\/dozer-core", optional = true }/; s/^digest = \["dep:sha2"\]/digest = ["dep:sha2", "dep:dozer-core"]/' crates/bytehost-apps/Cargo.toml
./scripts/check-bytehost-apps-deps.sh; echo "exit=$?"; cp $SCRATCH/Cargo.toml.bak crates/bytehost-apps/Cargo.toml
./scripts/check-bytehost-apps-deps.sh; git checkout -- Cargo.lock 2>/dev/null; git status --short
```
Expected: (a) 列出 `sha2`、`digest`、`generic-array` 等不在白名单里的 crate,`exit=1`;(b) 输出 `bytehost-apps 不得依赖 dozer* crate,发现:` 与 `dozer-core`,`exit=1`;还原后门禁 `ok`。**注意最后的 `git checkout -- Cargo.lock` 会把锁文件还原成没有新 crate 的版本——它只是清掉变异期间可能的改动;之后立刻重新生成:**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0 && cargo metadata --format-version 1 >/dev/null && git diff --stat Cargo.lock
```
Expected: `Cargo.lock | 11 +++++++++++`。

- [ ] **Step 8: 测试的变异检验(先暂存)**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0 && export PYTHONDONTWRITEBYTECODE=1
git add -A crates scripts Cargo.lock
cat > $SCRATCH/a0_mutate.py <<'EOF'
import sys
w = sys.argv[1]
S = "crates/bytehost-apps/src/"
M = {
 "1": (S+"id.rs", ".find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))", ".find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '.'))"),
 "2": (S+"permissions.rs", "escalation: to_n > from_n,", "escalation: to_n < from_n,"),
 "3": (S+"plan.rs", "        if self.plan.source_digest != source_digest {\n            return Err(VerifyError::SourceChanged);\n        }\n", ""),
 "4": (S+"state.rs", "            O::Failed {\n                retryable: true, ..\n            },\n        ) => Some(Action::Start),", "            O::Failed { .. },\n        ) => Some(Action::Start),"),
 "5": (S+"state.rs", "        | ObservedState::Running\n        | ObservedState::Stopping => ObservedState::Stopped,", "        | ObservedState::Stopping => ObservedState::Stopped,"),
 "6": (S+"registry.rs", "remove_dir_if_exists(&dir.join(\"cache\"))?;", "remove_dir_if_exists(&dir.join(\"cache\"))?;\n                remove_dir_if_exists(&dir.join(\"data\"))?;"),
 "7": (S+"digest.rs", "hasher.update(rel.as_bytes());", "let _ = rel;"),
 "8": (S+"manifest.rs", "|| path.split('/').any(|seg| seg == \"..\");", ";"),
 "9": (S+"manifest.rs", "digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())", "digest.len() >= 1"),
}
p, a, b = M[w]
s = open(p).read()
assert a in s, (w, a[:50])
open(p, "w").write(s.replace(a, b, 1))
EOF
for m in 1 2 3 4 5 6 7 8 9; do echo "M$m:"; python3 $SCRATCH/a0_mutate.py $m && cargo test -p bytehost-apps --all-features 2>&1 | grep -E "^error(\[|:)|FAILED$" | head -2; git checkout -- crates; done
git diff --stat | wc -l; git status --short | head -3
```
Expected: 九次变异各自让**对应**的测试失败:M1(`AppId` 放行 `.`):`app_id_rejects_everything_that_could_escape_a_hostname_or_a_directory`;M2(升权判反):`permissions::…fresh_install_diff…` 与 `event::…manifest_changed_reports_only_real_permission_differences`;M3(`verify` 不查源码摘要):`an_approved_plan_verifies_only_against_the_same_digests`;M4(不可重试的失败也自动重启):`a_non_retryable_failure_never_restarts_by_itself` 与 `next_action_truth_table`;M5(重启后 `Running` 不回 `Stopped`):`after_a_supervisor_restart_nothing_is_running_or_in_flight`;M6(卸载程序时连 `data/` 也删):`uninstalling_the_program_keeps_data_and_logs_and_a_reinstall_finds_them`;M7(目录摘要不含路径内容):`tree_digest_changes_with_content_name_addition_and_removal`;M8(路径校验不查 `..`):`paths_that_escape_the_package_are_rejected`;M9(镜像摘要只查非空):`container_images_must_be_pinned_by_digest`;每次还原后 `git diff --stat | wc -l` 为 `0`。

- [ ] **Step 9: 全 workspace 没被波及,然后提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0
cargo check -p dozer-core -p dozerd 2>&1 | grep -E "^error|Finished" | head -3
git status --short
git add crates/bytehost-apps scripts/check-bytehost-apps-deps.sh Cargo.lock
git diff --cached --stat | tail -14
git commit -m "feat(bytehost-apps): new headless crate — app model, manifest (TOML), install plan with digest-bound approval, lifecycle reconciliation, registry

First slice (A0) of the bytehost app host. Default features are empty (types and pure logic;
serde/serde_json only), manifest-toml adds Manifest::from_toml, digest adds SHA-256 and tree digests.
The crate depends on no dozer crate; scripts/check-bytehost-apps-deps.sh enforces that.

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: `cargo check` 以 `Finished` 结束;暂存区只有 `crates/bytehost-apps/**`(Cargo.toml + 9 个 `src` 文件)、`scripts/check-bytehost-apps-deps.sh`、`Cargo.lock`(11 行新增);**没有** `.cargo/`、`docs/`、`CLAUDE.md`。

---

### Task 2: 文档回填

**Files:**
- Modify: `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`、`docs/superpowers/specs/2026-10-04-bytehost-extraction-roadmap.md`、`CLAUDE.md`

**Interfaces:**
- Consumes: Task 1 的提交。

- [ ] **Step 1: 回填并校验**

用带引号的 heredoc 创建 `$SCRATCH/a0_docs.py`(提交短 id 经环境变量传入):

```python
import os
A0 = os.environ["A0"]
# 1) 应用宿主规格 §7 的 A0 行
sp = "docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md"
lines = open(sp).read().split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("| **A0** |"):
        lines[k] = l[:-1].rstrip() + " **已完成(A0,`bytehost-a0`):`" + A0 + "`**——manifest 用 TOML(用户 2026-10-04 裁决,`toml` 0.8,在 `manifest-toml` feature 下);默认 feature 为空、依赖仅 serde/serde_json;门禁 `scripts/check-bytehost-apps-deps.sh` |"
        hit = True
assert hit
s = "\n".join(lines)
a = "### 4.1 Manifest v1(最小)"
assert a in s
s = s.replace(a, a + "\n\n> **格式已定(2026-10-04,用户):TOML。** 下面的示例保留 YAML 写法只是为了与群聊原文对照;实际的 `manifest.toml` 见 `crates/bytehost-apps/src/manifest.rs` 里的测试样例(`[presentation]`、`[entrypoints.main]`、`[runtime]`、`[permissions.network]`…)。", 1)
open(sp, "w").write(s)
# 2) 路线图的 A0 行
rp = "docs/superpowers/specs/2026-10-04-bytehost-extraction-roadmap.md"
lines = open(rp).read().split("\n")
hit = False
for k, l in enumerate(lines):
    if l.startswith("- **A0** "):
        lines[k] = l + " **已完成(`bytehost-a0`):`" + A0 + "`。**"
        hit = True
assert hit
open(rp, "w").write("\n".join(lines))
# 3) CLAUDE.md 的 crate 表
cp = "CLAUDE.md"
s = open(cp).read()
a = "| `crates/dozer-mcp` | 面向外部 CLI agent 的只读 MCP stdio server（bin: `dozer-mcp`） |\n"
assert a in s
s = s.replace(a, a + "| `crates/bytehost-apps` | bytehost 应用宿主的无界面部分(应用模型 / 安装计划 / 生命周期 / 注册表),设计见 `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`。**不得依赖任何 `dozer*` crate**、默认 feature 的依赖只能是 serde 家族——门禁 `scripts/check-bytehost-apps-deps.sh` |\n", 1)
open(cp, "w").write(s)
```

Run:
```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0 && export PYTHONDONTWRITEBYTECODE=1
A0=$(git log --format=%h -1 --grep="new headless crate") python3 $SCRATCH/a0_docs.py
git diff -- docs CLAUDE.md | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)" | cut -c1-170
./scripts/check-bytehost-apps-deps.sh
```
Expected: `git diff` 里只有:应用宿主规格里 A0 行的"已完成"标记与 §4.1 开头的 TOML 说明、路线图 A0 行的"已完成"标记、`CLAUDE.md` 的 crate 表多一行 `crates/bytehost-apps`;**反引号内容完整**;门禁 `ok`。

- [ ] **Step 2: 提交**

```bash
cd ~/Projects/CoralProjects/byteboy/dozer-bytehost-a0
git status --short
git add docs CLAUDE.md
git diff --cached --stat | tail -5
git commit -m "docs(bytehost): backfill A0 (bytehost-apps crate; manifest format is TOML)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected: 只有 `docs/` 下 2 个文件与 `CLAUDE.md`。

---

## Self-Review

**1. 覆盖:** 应用宿主规格 §7 的 A0 项——manifest 解析与校验(`deny_unknown_fields`、`min_host_version`)✔、摘要 ✔、权限/授权/`InstallPlan`/`ApprovedInstallPlan` ✔、`AppState`(desired/observed)与对账纯函数 ✔、`AppEvent` ✔、registry/storage 文件读写 ✔、门禁 `cargo tree` ✔。规格 §4.2 的"授权与申请分存":`AppRecord.grants` 与 `Manifest.permissions` 是同一类型、不同存放位置 ✔(强制等级由 `PlanInput.enforcement` 带入计划)。**未覆盖并已在 Review Focus 8 说明:** 线上协议类型、`AppManager`、runtime adapter、gateway、`dozer-core`/`dozerd` 接线(A1/A2)。

**2. 占位符扫描:** 所有测试、实现、脚本都是草稿里跑通的完整版本(草稿在独立 worktree 里用同一批文件构建:默认/`digest`/`manifest-toml`/全部 feature 的测试数分别是 47/53/53/59,clippy 零诊断,门禁与 9 个变异均如上所述)。

**3. 一致性:** `AppId`/`Version`/`Permissions`/`Manifest`/`InstallPlan`/`ObservedState`/`AppRecord` 等名字与形状在测试、实现、Interfaces、文档里一致;`PlanInput` 用具名字段承载 7 个输入(避免相邻的 `String`/枚举参数顺序传错)。

**4. Review Focus:** 8 条各有归属(1、2、4、5、6→审阅/说明;3→`next_action_truth_table`;7→`unknown_fields_…`;8→范围说明)。

---

## 执行后修订(评审修复轮,2026-10-04)

独立评审(opus)对上面执行出的 crate 提出 6 条 Important,全部成立并在同一轮里修掉(每条先写失败测试再修;测试数 59 → **69**,默认 feature 47 → 55、`digest` 53 → 63、`manifest-toml` 53 → 61):

1. **`ApprovedInstallPlan::verify` 改为 `verify(&self, fresh: InstallPlan) -> Result<InstallPlan, VerifyError>`**:先比两个摘要,再比整份计划(新增 `VerifyError::PlanChanged`),并返回**重新计算的**计划——调用方安装/授予权限只能用返回值。原先只比两个摘要,线上 JSON 里被改宽的 `requested`/`enforcement`/`permission_diff` 会原样放行。同时订正了"字段私有、只能经 `approve` 构造"的过度表述(它可以被反序列化出来,安全属性只来自 `verify`)。
2. **`digest_tree` 拒绝非 UTF-8 文件名**(原先 `to_string_lossy` 会让 `a\xff` 与 `a\u{FFFD}` 变成同一个键,载荷文件的字节可能根本没进摘要)。
3. **`digest_tree` 只接受普通文件与目录**(FIFO/套接字/设备返回 `InvalidInput`,原先 FIFO 会让安装任务永久阻塞);模块文档补写了"摘要覆盖整个包根目录"与"防 TOCTOU 的前提是对 staging 副本算摘要再 `verify`"。
4. **`next_action(Stopped, Failed{..})` 改为 `Some(Stop)`**(规格 §4.4:Failed 可经 stop 复位;失败的进程型应用可能还留着活进程)。
5. **`AppRecord` 新增 `format_version` 与 `observed`**,`state.json` 读到不认识的版本直接拒绝(`RECORD_FORMAT_VERSION = 1`);**被打断的 `Updating` 恢复成 `Stopped`**(每个版本的包目录不可变、`current_version` 最后才写,旧版本仍然完整),原先的"可重试失败"在 `desired = Running` 下会立刻 `Start`、既不重做升级也不检查包。`load` 与 `list` 共用一个 `parse_record`。
6. **`Manifest::validate` 补了面向 A2/A4 的约束**:入口路径与 `health.path` 不得以 `//` 开头、不得含反斜杠/`?`/`#`/控制字符/`..` 段;`name`、入口 `title`、命令参数不得含控制字符或双向控制字符;容器镜像名不得以 `-` 开头、不得含空白。`will_run` 对命令参数做 shell 风格引用并转义控制/双向字符(`["python","-m app"]` 与 `["python","-m","app"]` 不再渲染成同一行)。

**推迟的 Minor(11 条)**记在 `docs` 之外的执行账本里,交给用户决定;其中值得 A1 前处理的:门禁脚本只查 `-e normal` 的宿主目标(漏 build-dependency 与非 mac 的 cfg 依赖)、`write_atomic` 无 `fsync`、`next_action` 对"可重试失败"没有退避上限(A1 的 manager 必须自己限制)。
