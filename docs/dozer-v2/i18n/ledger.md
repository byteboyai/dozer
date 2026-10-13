# Dozer i18n 任务总账

> 日期：2026-10-12。对应计划 `2026-10-12-dozer-i18n-implementation.md`。
> 本文件跟踪任务级进度与扫描总账；逐条候选状态在 `inventory.json`。
> 总账恒等式（计划 §1.3）：**候选总数 = 已迁移 + 有理由排除 + 未解决**。

## 1. 扫描总账（Task 01 基线）

| 项 | 数量 |
|---|---|
| 候选总数 | 4208 |
| 已迁移（migrated） | 0 |
| 有理由排除（excluded） | 3001 |
| 未解决（unresolved，UI 待迁移） | 1207 |

复算：`0 + 3001 + 1207 = 4208` ✅

- 排除理由分布：测试/fixture、机器日志、开发期诊断宏、内部工具/黄金值、后端 crate 非 UI 字符串（详见 `inventory.json` 各条 `reason`）。
- 重跑方式：`python3 scripts/i18n/scan_sources.py --json /tmp/i18n_candidates.json` 后 `python3 scripts/i18n/build_inventory.py`。
- `inventory.json` 每次覆盖重写，禁止手改结构；未解决数变化须与扫描复核一致，禁止删基线记录让数字归零。

## 2. 未解决 UI 候选按任务分配

| Task | 主题 | 未解决候选 |
|---|---|---|
| 05 | 设置入口与面板标题 | 50 |
| 06 | Host chrome/菜单/原生 overlay | 282 |
| 07 | Toast 与错误呈现 | 102 |
| 08 | WebView locale 传递基础 | 8 |
| 09 | Todo / Usage / Code Health 首批面板 | 240 |
| 10 | 其余官方面板与辅助界面 | 413 |
| 11 | 编辑/预览 Web 与 HTML 模板 | 112 |
| **合计** | | **1207** |

未分配的 UI 区域：**0**（生成器 `build_inventory.py` 在存在无任务候选时报警并列出）。

## 3. 未解决 UI 候选按 owner 分布

| owner | 数量 |
|---|---|
| dozer.host | 422 |
| dozer.codehealth | 153 |
| dozer.project | 122 |
| dozer.group_chat | 92 |
| dozer.preview | 91 |
| dozer.database | 69 |
| dozer.files | 62 |
| dozer.todo | 60 |
| dozer.ssh | 34 |
| dozer.git_log | 27 |
| dozer.usage | 27 |
| dozer.web | 20 |
| dozer.agent | 15 |
| dozer.conversations | 13 |

## 4. 任务进度

| Task | 状态 | 证据 | 残留 |
|---|---|---|---|
| 01 清单与生产入口基线 | ✅ 完成 | 本文件、`baseline.md`、`glossary.md`、`inventory.json`、`scripts/i18n/` | 迁移尚未开始，见 `acceptance.md` |
| 02–05 | 未开始 | — | 依赖 01 |
| 06–08 | 未开始 | — | 依赖 05 |
| 09–11 | 未开始 | — | 依赖 06/07/08 |
| 12–14 | 未开始 | — | 见计划 §2 |

## 5. 门禁与工具

| 工具 | 路径 | 用途 |
|---|---|---|
| 候选扫描 | `scripts/i18n/scan_sources.py` | 提取 CJK 候选并分类（Task 01） |
| 清单生成 | `scripts/i18n/build_inventory.py` | 生成 `inventory.json`、校验任务分配（Task 01） |
| 面板边界 | `scripts/audit/check_panel_boundary.py` | 既有；面板不得新增 client 依赖 |
| 资源校验 | `scripts/i18n/check_resources.py` | 待 Task 02 新增 |
| 迁移门禁 | `scripts/check-i18n.sh` | 待 Task 12 新增 |

## 6. 完成定义提醒

- 单有 locale 设置、翻译函数或两份 JSON 文件，均不算完成多语言迁移（计划 §5）。
- 每条存量用户可见中文须有可追踪结论：迁入 `zh-CN.json` 的 key，或有明确非 UI 理由的保留项。
- 迁移完成 = 资源存在 + 代码调用已替换 + 中文显示经核对 + 英文翻译存在，四者同时成立。
