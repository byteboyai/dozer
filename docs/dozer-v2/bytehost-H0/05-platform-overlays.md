# H0-05:`platform/` overlay 迁移清单

> 基线提交:`108f46b`(bytehost-h0 分支,代码与 `main` 的 `4845fb85` 一致)。
> 机械数据:`data/platform.md`,生成命令 `python3 scripts/audit/report.py platform`。
> 回答:规格 §10 H0 范围第 4 项(`platform/` 下各面板专属 overlay 的迁移清单)。登记清单:`E3-registry.tsv` 的 `overlay` 类(E3-025…E3-038)。

## 1. 总览

`crates/dozer-app/src/platform/` 有 22 个文件(含 `mod.rs`)。机械判断(只看是否引用 `extensions::*`)把它们分成:通用候选 7 个(`file_drag`、`overlay_focus`、`overlay_gpu`、`overlay_window`、`picker`、`window`、`mod`)、面板专属 10 个、混合 4 个(`confirm_overlay`、`file_history_overlay`、`settings_overlay`、`window_events`)、`toast_overlay`(引用 `toast`,而 `toast` 是 host 基础设施)。人工逐个读文件头与引用点后的结论如下。

**一个结构性事实:** `window_events.rs` 单文件 **4602 行**,是 winit `ApplicationHandler` 的实现,既是 host 事件循环,又内联了对 Files、Browser、Project、ProjectCreate、Todo、Search、GroupChat 的处理(见 `03-host-to-panel.md` E3-010/E3-011)。它比所有 `*_overlay.rs` 加起来还大,是 `platform/` 里最大的单点。

## 2. 逐文件表

| 文件 | 行 | 机械建议 | **人工裁决** | 依据 | E3 |
|---|---|---|---|---|---|
| `overlay_window.rs` | 299 | 通用 | **host** | 文件头:独立原生窗口共用的建窗样板与居中定位,供 search/file_history 及"未来消费方"共用;只引用 `theme`×1 | — |
| `overlay_gpu.rs` | 287 | 通用 | **host** | 独立原生窗口共用的 wgpu 管线建立/重配置 | — |
| `overlay_focus.rs` | 56 | 通用 | **host** | 共用的"是否该因失焦关闭"判定(含 winit 合成 `Focused(false)` 的处理) | — |
| `picker.rs` | 36 | 通用 | **host(macOS)** | 文件/目录选择单入口;规格 §3.1"文件选择、拖放等系统接入" | — |
| `file_drag.rs` | 165 | 通用 | **host(macOS)** | 补 winit 缺的 `draggingUpdated:`;文件树高亮命中是它的消费方,但机制本身无面板依赖 | — |
| `window.rs` | 325 | 通用 | **host(macOS)** | 交通灯居中 + 顶栏拖窗守卫 | — |
| `mod.rs` | 25 | 通用 | **host** | 模块声明 | — |
| `toast_overlay.rs` | 275 | 面板专属:toast | **host** | `toast` 是 host 基础设施(规格 §3.1),项目 CLAUDE.md 已把 Toast 定为统一机制 | — |
| `confirm_overlay.rs` | 275 | 混合 | **拆分** | 弹窗宿主本身通用("通用标题+说明+取消/确认弹窗",文件头);但 `:176-196` 逐面板收集确认规格并提升消息类型(`files::delete_confirm_spec`、`database::delete_confirm_spec`、`ssh::delete_confirm_spec`、`todo::clear_confirm_spec`、`map_todo_spec`) | E3-037 |
| `settings_overlay.rs` | 256 | 混合 | **拆分:壳留 host,内容留产品** | 设置机制(注册/持久化/展示)归 host,内容(主题 + Git 账户)是产品;`:31-36` 引用 `project_create::card_logical_size` 复用卡片尺寸,应下沉成通用助手 | E3-034 |
| `search_overlay.rs` | 415 | 面板专属:search | **随 search 走** | 引用 `runtime`×3、`event`×1;它是 `overlay_window`/`overlay_gpu` 的原始来源,共享机制已抽走,剩下的是 search 专属 | E3-033 |
| `file_history_overlay.rs` | 462 | 混合 | **随 file_history 走** | 引用 `file_history`×1、`diff_content`×2;`diff_content` 是 file_history 与 git_log 共用的 diff 内容层(bytegit P2 抽出),应随 diff/历史类面板 | E3-028 |
| `edit_history_overlay.rs` | 506 | 面板专属 | **随 edit_history 走,依赖 Preview** | 引用 `preview`×10、`assets`×2:这个弹窗渲染预览内容,绑定 Preview 业务,**不能先于 Preview 抽象单独迁** | E3-027 |
| `files_move_overlay.rs` | 190 | 面板专属:files | **随 Files Tree 走** | 文件树拖拽移动确认;注意文件头"不接失焦关闭"——它依赖 `MoveDirBrowse` 同步弹 rfd | E3-029 |
| `project_create_overlay.rs` | 269 | 面板专属 | **随 Project 走** | 创建项目对话框 | E3-030 |
| `project_delete_overlay.rs` | 180 | 面板专属 | **随 Project 走** | 删除项目三选一 | E3-031 |
| `project_scaffold_overlay.rs` | 165 | 面板专属 | **随 Project 走** | 修复项目进度弹窗,`scrim_blocking` 语义 | E3-032 |
| `database_drivers_overlay.rs` | 177 | 面板专属 | **随 Database 走** | | E3-025 |
| `database_source_overlay.rs` | 217 | 面板专属 | **随 Database 走** | | E3-026 |
| `ssh_host_overlay.rs` | 209 | 面板专属 | **随 Ssh 走** | | E3-035 |
| `todo_detail_overlay.rs` | 193 | 面板专属 | **随 Todo 走** | | E3-036 |
| `window_events.rs` | 4602 | 混合 | **拆分(最大单点)** | host 事件循环状态机 + 各面板事件处理 | E3-038(及 E3-010、E3-011) |

## 3. 对规格的一个修正

规格 §3.3 写"具体面板的分支及其专属 overlay"留在 Dozer 产品层。实测面板专属的 overlay 宿主有 **11 个,其中 5 个属于 Digger 计划复用的面板**(Files Tree 1 个、Project 3 个、Todo 1 个)。这意味着:**专属 overlay 必须随面板一起走(跟着面板被共享),不能简单"留在 Dozer 产品层"**,否则复用面板在 Digger 里没有对应弹窗。建议规格 §3.3 改成"overlay **宿主壳**留面板(随面板迁出),overlay **窗口机制**(`overlay_window`/`overlay_gpu`/`overlay_focus`)归 host"。

## 4. 迁移顺序建议

1. **先稳定 host 机制:** `overlay_window`/`overlay_gpu`/`overlay_focus` 已是共享机制(2026-09-18 迁移),不需要动;只需确认它们的公开 API 不出现业务类型(E2)。
2. **`confirm_overlay.rs:176-196`:** 把逐面板收集确认规格改成"面板经 registry 提供 `ConfirmDialog` 规格"(E3-037),是 overlay 里最小、最独立的一刀。
3. **`window_events.rs` 拆分:** 是 overlay 迁移的前置——只要它还内联面板事件处理,面板就迁不出去。建议先把 E3-010/E3-011 的"面板事件"改成面板声明式钩子(依赖 `01-panelkind.md` B4 与规格 O12 的验收夹具)。
4. **各 `*_overlay.rs` 随面板迁出:** 无独立顺序,跟随各面板的迁出批次;`edit_history_overlay` 要在 Preview 业务边界稳定之后。
5. **`settings_overlay.rs`:** 先把 `card_logical_size` 下沉成通用助手,再决定壳与内容的拆法。
