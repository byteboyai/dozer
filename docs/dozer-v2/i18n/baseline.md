# Dozer i18n 基线（Task 01）

> 日期：2026-10-12。本文件记录实施多语言迁移前的代码、构建与工具链基线。
> 对应计划：`docs/superpowers/plans/2026-10-12-dozer-i18n-implementation.md` Task 01。
> 规格：`docs/superpowers/specs/2026-10-12-dozer-i18n-design.md`。

## 1. 代码 revision 与工作区状态

- HEAD：`f7fb0605ada8d07123b4cc35cb6006795c8ce5e7`（`f7fb0605`，"feat(files): 文件树多选拖拽/删除 + 会话摘要长度调整"）。
- 基线记录时的工作区改动：
  - `Cargo.lock`、`crates/dozer-app/Cargo.toml`、`crates/dozer-app/packaging/macos/Info.plist`：打包脚本自动版本号提升（0.5.291→0.5.292，CFBundleVersion 321→322），非本任务改动。
  - `docs/superpowers/specs/2026-10-12-dozer-i18n-design.md`：本任务 §5 资源路径修订（见 §4）。
  - 新增 `docs/dozer-v2/i18n/`、`scripts/i18n/`：本任务交付物。
- 保留其他任务 WIP，未重置或覆盖。

## 2. 工具链版本

| 工具 | 版本 |
|---|---|
| rustc | 1.98.1 (48a229cea 2026-09-01) |
| cargo | 1.98.1 |
| node | v24.14.0 |
| npm | 11.11.0 |
| python3 | 3.13.1 |

> 注：本机 shell 把 `find` 别名为 `fd`；脚本与文档统一使用 `fd`/glob，不用 `find`。

## 3. 构建与验证入口

- 全 workspace 构建：`cargo build`；GUI：`cargo run -p dozer-app`。
- 打包：`scripts/build-macos-app.sh`（release 构建 + 自动版本号提升，产出 `/Applications/Dozer AI Coder.app`，含 `dozer`/`dozer-hook`/`dozer-mcp`/`dozerd`）。
- 审计脚本：`python3 scripts/audit/check_panel_boundary.py`（面板依赖边界门禁）。
- 现有已知失败（与本次无关，基线保留）：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`（引用 `app/update.rs:5318` 已过期）。

## 4. 资源所有者与路径

规格 §5 的资源路径提案已按当前代码修订：

```text
crates/dozer-app/locales/host/{en,zh-CN}.json
crates/dozer-app/locales/panels/<panel-id>/{en,zh-CN}.json   # 仍在 app 内的面板
crates/dozer-panel-usage/locales/{en,zh-CN}.json             # 已独立的面板随其 crate
crates/dozer-app/web/shared/i18n/       # TS 解析器与一致性测试
crates/dozer-i18n/                     # Rust 解析器与一致性测试
```

- 面板 `PanelKind`：`files`/`todo`/`project`/`database`/`ssh`/`web`/`agent`/`conversations`/`usage`/`code_health`/`group_chat`。
- owner 命名：Host = `dozer.host`；面板 = `dozer.<panel>`；预览 chrome = `dozer.preview`。
- **Usage 已迁出**：当前 `Usage` 的唯一生产入口是独立 crate `crates/dozer-panel-usage/`（`src/`、`web/usage-content/`、`assets/usage-content/`）。其 locale 资源归属 `crates/dozer-panel-usage/locales/`，不得反向依赖 `dozer-app`。本任务确认该结构，不恢复已删除目录，不维护两份 Usage 译文。

## 5. Web 包清单（自有前端）

`dozer-app/web/*` 与独立面板前端，均需在对应 `package.json` 目录运行 `npm run typecheck` / 已有或新增 `npm test` / `npm run build`：

| 包 | 路径 | 备注 |
|---|---|---|
| todo-content | `crates/dozer-app/web/todo-content/` | 自有 envelope + Surface |
| codehealth-content | `crates/dozer-app/web/codehealth-content/` | Rust 侧已拼中文，需追加语义字段 |
| group-chat-content | `crates/dozer-app/web/group-chat-content/` | 转义渲染器；构建产物一并提交 |
| editor | `crates/dozer-app/web/editor/` | CodeMirror |
| json-editor | `crates/dozer-app/web/json-editor/` | 无 test script |
| tabular-host | `crates/dozer-app/web/tabular-host/` | 无 test script |
| review-trace | `crates/dozer-app/web/review-trace/` | |
| plantuml-viewer | `crates/dozer-app/web/plantuml-viewer/` | build 含 scan-offline |
| flyfish | `crates/dozer-app/web/flyfish/` | 额外 `npm run build:check` |
| usage-content | `crates/dozer-panel-usage/web/usage-content/` | 随独立 crate |

## 6. 扫描基线（首次全量）

- 工具：`scripts/i18n/scan_sources.py`（本任务新增）。覆盖 Rust `"…"`/`r#"…"#`、`format!`、TS/TSX/JS 字符串与模板字面量、JSX 文本、HTML 属性。
- 分类：`ui`（需迁移）/`comment`（排除）/`log`（排除）/`diag`（排除，含 assert/panic 宏）/`test`（排除，随协议迁移者除外）。
- 首次全量结果见 `inventory.json`；总账满足「候选总数 = 已迁移 + 有理由排除 + 未解决」。
- 稳定性：候选以 `cand-<sha1(path + offset + kind)[:12]>` 为稳定 ID，行号仅作定位辅助，不作身份。

## 7. 残留问题与当前未审计项

- 尚未实施迁移，`inventory.json` 中 `migrated = 0`，官方两包不存在；所有 UI 候选保持 `unresolved`。
- 面板目录未新增语言包；`report` 只登记盘点结论，不宣称任何区域已支持双语。
- 本基线不改变 fallback 或资源共享语义；具体解析器、持久化与热切换在 Task 02 起实施。
