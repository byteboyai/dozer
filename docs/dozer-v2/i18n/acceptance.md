# Dozer i18n 验收记录

> 日期：2026-10-12。逐任务记录命令/证据/结果；最终按规格 §12 L01–L14 逐条验收（Task 13）。
> Task 01 只交付基线，尚未实施功能迁移；本文件当前只记录 Task 01 验收与整体待办。

## Task 01 验收

计划 Task 01 验收条件：**无未分配的 UI 区域；候选有分类/负责人任务；未迁移清单可重跑。尚未审计项保持 unresolved。**

| 检查项 | 命令/证据 | 结果 |
|---|---|---|
| 无未分配的 UI 区域 | `python3 scripts/i18n/build_inventory.py` 未列出无任务候选 | ✅（unassigned unresolved = 0） |
| 候选有分类与 owner | `inventory.json` 每条含 `usage`/`owner`/`task`/`status`/`reason` | ✅ |
| 候选有任务 | `inventory.json` `unresolved_by_task` 覆盖全部未解决项 | ✅ |
| 未迁移清单可重跑 | 重跑扫描 + 生成器，数字与 `ledger.md` §1 一致 | ✅ |
| 未审计项保持 unresolved | `inventory.json` 中无候选被标 `migrated` | ✅ |
| 总账恒等式 | `0 + 3001 + 1207 = 4208` | ✅ |

### 重跑命令

```bash
python3 scripts/i18n/scan_sources.py --json /tmp/i18n_candidates.json
python3 scripts/i18n/build_inventory.py
```

### 交付物

- `docs/dozer-v2/i18n/baseline.md`
- `docs/dozer-v2/i18n/inventory.json`
- `docs/dozer-v2/i18n/glossary.md`
- `docs/dozer-v2/i18n/ledger.md`
- `docs/dozer-v2/i18n/acceptance.md`（本文件）
- `scripts/i18n/scan_sources.py`、`scripts/i18n/build_inventory.py`
- 规格 §5 资源路径修订（Usage 独立 crate 归属）

### 残留问题

- 迁移尚未开始（`migrated = 0`），官方语言包不存在；所有未解决 UI 候选待各自任务处理。
- 扫描器以静态字面量为主；无静态字面量但可显示文本的入口（如 Rust 拼接 payload、服务 message）由各迁移任务沿调用链人工补登。
- 既有无关测试失败保留：`usage_probe::host::tests::host_site_citations_point_at_the_code_they_name`。

## Task 02 验收

计划 Task 02 验收条件：**Rust/TS 共享向量同结果；注入缺 key/参数不一致会检查失败；中文展示确实来自 zh-CN 资源。对应 L05/L06。**

| 检查项 | 命令 | 结果 |
|---|---|---|
| Rust/TS 共享向量同结果 | `cargo test -p dozer-i18n --test vectors` + `node crates/dozer-app/web/shared/i18n/i18n.test.ts` | ✅ 两读 `vectors/interpolation.json`，同结果 |
| 缺 key 检查失败 | `python3 scripts/i18n/check_resources.py <临时目录>` | ✅ 注入缺 key/多余 key → FAIL |
| 参数不一致检查失败 | 同上（占位符集不同） | ✅ FAIL |
| 重复 key 覆盖前发现 | 同上（`object_pairs_hook`） | ✅ FAIL |
| 中文来自 zh-CN 资源 | `cargo test -p dozer-app i18n::` | ✅ `settings.title`→「设置」来自 `zh-CN.json`，非 fallback |
| 解析器一致性单测 | `cargo test -p dozer-i18n` | ✅ 18 单测 + 1 向量测试通过 |

### 交付物

- `crates/dozer-model/src/i18n.rs`：`Locale`/`LocalePreference`/`LocaleSnapshot`/`LocalizedText` + 具名参数。
- `crates/dozer-i18n/`：`bundle.rs`(装载/校验/parity)、`interp.rs`(插值)、`locale.rs`(规范化)、`lib.rs`(Resolver/fallback)、`vectors/interpolation.json`、`tests/`。
- `crates/dozer-app/locales/host/{en,zh-CN}.json`：最小 Host 词表。
- `crates/dozer-app/src/i18n.rs`：编译期嵌入 + `host_resolver()` 接缝。
- `crates/dozer-app/web/shared/i18n/i18n.ts`、`i18n.test.ts`：中立共享 TS 解析器。
- `scripts/i18n/check_resources.py`：资源完整性检查。

### 残留问题

- 面板标题尚未接入 `PanelDescriptor`/`product.rs`/设置页（Task 04/05）。
- 复数变体（`.zero`/`.one`/`.other`）由调用点选择，解析器不内建 ICU；具体规则见 spec §6.2 与 `glossary.md` §5。
- TS 解析器目前无包内 `npm test` 接线（各 Web 包接线在 Task 08/11）。

## 规格 L01–L14 验收（Task 13 执行，当前未开始）

| ID | 场景 | 状态 |
|---|---|---|
| L01 | 系统语言匹配（en-US / zh-Hans-CN / zh-TW / 未知 / 空） | ⬜ 待实施 |
| L02 | 选择 zh-CN 后重启 GUI/daemon，再改 OS 语言 | ⬜ 待实施 |
| L03 | 配置缺失/损坏/写入失败/服务离线 | ⬜ 待实施 |
| L04 | 两窗口/两进程提交冲突，断线重连 | ⬜ 待实施 |
| L05 | 缺 key/空翻译/重复 key/错参数/未知 owner | ⬜ 待实施 |
| L06 | Rust/TS 共享测试向量（Unicode/花括号/恶意 HTML） | ⬜ 待实施 |
| L07 | 有编辑/滚动/终端/任务时切换，工作状态保留 | ⬜ 待实施 |
| L08 | 所有 overlay/菜单/现存 Toast | ⬜ 待实施 |
| L09 | WebView ready 前切换/隐藏后切换/重建/旧包晚到 | ⬜ 待实施 |
| L10 | Code Health/Usage 计数、日期、金额及服务失败 | ⬜ 待实施 |
| L11 | en/zh-CN + 明暗主题 + 最小窗口 + 缩放，CJK 无方块 | ⬜ 待实施 |
| L12 | 新旧 GUI/daemon/前端配对，旧第三方应用 | ⬜ 待实施 |
| L13 | MCP/日志/存储快照切换前后对比 | ⬜ 待实施 |
| L14 | 单面板 + Host 测试组合 | ⬜ 待实施 |
