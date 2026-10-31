# ByteUI 跨产品优化验证基线

日期：2026-10-31。对应计划：`docs/superpowers/plans/2026-10-31-byteui-optimization.md`（P0）。
本文区分**已核对事实**与**未执行的验证**；未执行的项不写成通过。

## 1. 环境与仓库状态

采集时间：2026-10-31（实施开始前）。

| 项 | 值 |
| --- | --- |
| 主机 | Apple M3 Max，macOS 26.6.2 (25G83)，arm64 |
| Rust | rustc 1.98.1 (48a229cea 2026-09-01) |
| Cargo | cargo 1.98.1 (797e8a9bc 2026-08-05) |
| 工作目录 | `/Users/chrischiang/Projects/CoralProjects/byteboy` |

| 仓库 | HEAD | 分支 | 实际 tag | 采集时工作区 |
| --- | --- | --- | --- | --- |
| ByteUI | `1285b9ac362e6eeaf1d4fa8a9caef7fd3ee62bbc` | main | `v0.5.0` | 仅未跟踪 `.dozer/` |
| ByteHost | `324593d116520ecfab4de7be8ceccb081acbc933` | main | `v0.4.19-1-g324593d` | clean |
| Dozer | `d36c1562858a013db6ef378be479c5c3ee359046` | main | `libs-2026-10-28g-32-gd36c1562` | 仅本计划相关未跟踪文档 |
| Digger | `138e3ff3315f251e9597521c6070910e1712a595` | main | 无 tag（`138e3ff`） | clean |

### 1.1 锁文件来源（事实）

Dozer `Cargo.lock`：

- `byteui 0.5.0` ← `git+https://github.com/byteboyai/byteui?tag=v0.5.0#1285b9ac…`
- `bytehost-*` ← `git+https://github.com/byteboyai/bytehost?tag=v0.4.20#8cb6eed0…`
- `bytegit 0.8.1` ← `tag=v0.8.1#ae60c831…`

Digger `Cargo.lock`（采集时）：

- `byteui 0.5.0` ← `tag=v0.5.0#1285b9ac…`
- `bytehost-*` ← `tag=v0.4.20#8cb6eed0…`
- `dozer-*` ← **没有 `source`**（上一次生成锁文件时被本地 path patch 覆盖）。

### 1.2 本地 patch（不入库）

- `dozer/.cargo/config.toml`：把 `byteui` 指到 `../byteui`，把 `bytegit` 指到 `../bytegit`。
- `digger/.cargo/config.toml`：把全部 `dozer-*` 指到 `/Users/.../dozer/crates/*`。
- 两个文件都被各自 `.gitignore` 忽略，不属于发布组合。

### 1.3 P3 重复依赖问题（已解除）

在**不加载 Digger 本地 patch** 的情况下重新解析时曾发现：

- `libs-2026-10-28g#b1c08678` 内的共享面板仍声明 `bytehost v0.4.17#87f1c28`，
  与 Digger 直接使用的 `v0.4.20#8cb6eed0` 形成两套 `bytehost-*`。
- 解除方式：Dozer 已有对齐后的库快照 tag `libs-2026-10-30a#7f41bcbe`
  （其 `Cargo.toml` 钉 `bytehost v0.4.20`）。Digger 的 16 个 `dozer-*` 依赖
  已从 `libs-2026-10-28g` 升到 `libs-2026-10-30a`，无 patch 重新生成锁文件后：
  - `bytehost-*` 只剩 `v0.4.20` 一份；
  - `cargo metadata --locked` 退出码 0；
  - `scripts/check-shared-deps.sh --offline` 输出“没有发现问题”。
- 仍待完成：用**新发布的 ByteUI tag** 替换 Digger 当前的 `byteui v0.5.0`
  后才能做最终 `cargo check --locked --workspace --all-targets`；这是正常的
  发布顺序（ByteUI → ByteHost → Dozer 快照 → 两产品），不再是依赖图冲突。

Dozer 自身在不加载本地 patch 时 `cargo metadata --locked` 可解析（退出码 0），
锁文件与 workspace 声明一致。

## 2. 现有守卫盘点（复用，不重复实现）

| 守卫 | 位置 | 当前覆盖 |
| --- | --- | --- |
| `cargo fmt --check` | ByteUI / Dozer / Digger CI | 格式 |
| `cargo clippy --all-targets -- -D warnings` | ByteUI CI | ByteUI lint |
| `cargo test` | ByteUI CI | 单元测试 |
| `cargo check --locked --workspace --all-targets` | Dozer CI | 发布组合编译 |
| `cargo check --workspace --all-targets` | Digger CI（本次改为 `--locked`） | 发布组合编译 |
| `scripts/check-host-deps.sh` | 两产品 | 业务层不得直连宿主层 |
| `scripts/check_shared-deps.sh` / `check_shared_deps.py` | 两产品 + ByteHost 正本 | 共享库 tag/rev 一致性、锁文件来源、重复来源 |
| `scripts/audit/check_panel_boundary.py` | Dozer | 面板边界棘轮 |
| ByteUI consumer workflows | 两产品 | 仅编译与上述守卫 |

本次新增覆盖：

- `check_shared_deps.py`：清单声明 ⇔ `Cargo.lock` 实际 ref 一致性；iced 全家桶
  版本唯一且为 0.14.x（从锁文件读取，不比较清单字符串）。
- ByteUI `.github/workflows/candidate-check.yml`：候选四仓 SHA 的临时 patch +
  临时锁 + locked 构建 + 解析图留档。
- Dozer 外壳比较脚本已退役（见 P4），不再要求独立 checkout 拉 sibling 源码。

## 3. 菜单生产调用路径（事实）

ByteUI API：`install_content_view` 登记线程局部裸 `NSView` 指针；`show` /
`show_align_no_icon_left` 读取该登记并同步弹菜单。

Dozer（`dozer-app`）：

- 主窗登记：`platform/window_events.rs`（窗口创建时）。
- 浮窗登记 + Drop 恢复主窗：`platform/{search,settings,project_create,group_create,database_source,ssh_host,files_move,todo_detail}_overlay.rs`。
- `show` / `show_align_no_icon_left` 调用点：`app/update.rs`（约 14 处）、
  `app/app.rs`（3 处）、`host_adapters/conversations_panel.rs`（1 处）。
- 菜单数据构造：`menu_spec.rs`、`workspace/view.rs`、`chrome/topbar.rs`、
  `extensions/{files,database,ssh}/…`、`app/app.rs`。

Digger（`desktop`）：

- 登记：`product.rs` 的窗口就绪回调（`window::run` 内）。
- `show` 调用：`product.rs`（4 处）、`adapters/conversations.rs`、
  `adapters/browser.rs`、示例 `examples/conversations_gallery.rs`。
- 菜单数据：`adapters/{native_menu,conversations_search_menu,browser_addr_menu}.rs`。

本次已把全部生产调用迁移到 `byteui::native_menu::show_for_window` /
`show_align_no_icon_left_for_window`：

- Dozer `App` 保存 `Weak<Window>`；主窗事件与每个浮窗事件处理前更新，浮窗销毁后
  `upgrade()` 失败，迟到菜单请求返回 `None`。
- Digger 菜单改为 `window::run(id, |handle| show_for_window(...))`，选中项作为
  `Task` 投回产品 `update`；gallery 同步迁移。
- 旧 `install_content_view` / `show` / `show_align_no_icon_left` 已从 ByteUI 删除。
- 已覆盖空菜单/无 AppKit 句柄返回 `None`；真实多窗口交互未人工验证。

## 4. 主题 setter 与网页更新路径（事实）

ByteUI：

- `color::set_theme` / `set_scheme` / `persist_scheme` / `init_scheme`。
- `font::set_theme`、`geometry::set_theme`、`icon_size::set_theme`。

Dozer：

- 启动：`main.rs` 调 `theme::init()`（font/geometry/icon_size）与
  `color::init_scheme`。
- 设置：`extensions/settings.rs` 的 `ThemeSelected` 调 `set_scheme` 并落盘；
  `app/update.rs` 在设置更新后重载 Files/Project 预览 webview。
- 第一方网页：`preview/webview.rs::scheme_query_value` 生成 `theme=` 参数；
  Usage/Todo/CodeHealth 在 `app/app.rs` 拼 `host.html?theme=…`。

Digger：

- 启动/设置：`settings/theme.rs::init_notice` / `select` / `apply` → `set_scheme`。
- `ThemeSelected` 之后：preview `reload_for_theme`、review native `sync_theme`、
  markdown `sync_theme`。
- 第一方网页：`settings/theme.rs::code` / `review_script`；todo/usage/group-chat
  native 适配层读取 `code()`。

本次把颜色与方案名合并为单锁 `ThemeSnapshot { colors, scheme, source, revision }`；
Digger 的 `review_script` 与 Dozer 的 `scheme_query_value` 改为从同一快照生成。
revision 已暴露，但**尚未接入异步网页投递的过期丢弃**（现有投递是同步重建，
不构成已知回退，标记为剩余项）。

## 5. 外壳样式基线（迁移目标）

ByteUI `theme::region`（scale = 1.0，Dark）：

| 区域 | background | border(color/width/radius) | padding | gap | margin |
| --- | --- | --- | --- | --- | --- |
| top_bar | `BG` | `BORDER` / 1 / 0 | 0,10,0,78 | 8 | 0 |
| left_icon_rail | `PANEL` | 无 | 16,6,0,6 | 12 | 0 |
| right_icon_rail | `PANEL` | 无 | 16,6,0,6 | 12 | 0 |
| status_bar | `PANEL` | `BORDER` / 1 / 0 | 0,8 | 0 | 0 |
| left_zone | `BG` | `BORDER` / 0 / 8 | 1 | 0 | 6,0,0,6 |
| right_zone | `BG` | `BORDER` / 0 / 8 | 1 | 0 | 6,6,0,0 |
| context_menu | `PANEL`(alpha 1) | `BORDER` / 1 / 9 | 8 | 1 | 0 |
| maximize_overlay | scrim `SCRIM`, padding 40 | `GOLD` / 1.5 / 10 | — | — | — |
| window background | `TAB_ACTIVE_BORDER` | — | — | — | — |

缩放规则：padding/gap/margin 乘全局 scale；普通区域描边不缩放；maximize overlay
的 scrim padding 与描边宽/圆角缩放（ByteUI 测试锁定该差异）。

本次迁移后 Dozer 共用 accessor 全部委托 ByteUI；`workspace.json` 只保留 8 个
产品面板区（project/preview/browser/agent/terminal/conversations/group_chat/review），
不再有顶层 `background`。迁移前后对比测试：`cargo test -p dozer-app theme::region`
（14 passed）。

## 6. 动画测量方法与数据

测量方法（尚未执行真实窗口采样）：

- 构建模式：`cargo run --release`（同机同构建模式比较前后）。
- 场景：0 / 1 / 4 个 `view_animated` 实例；面板可见、隐藏、窗口最小化。
- 窗口：默认 1440×900，scale 1.0；另测 0.5 / 2.0。
- 采样：每场景 ≥ 30 秒；同时记录实际 `RedrawRequested` 数、进程 CPU
  （`ps -o %cpu` 或 Activity Monitor）、可取到的帧时间（`Sample`/GPU 工具）。
- 限制：没有窗口内 instrumentation 时不能把“请求重绘数”等同“实际呈现帧数”；
  工具不可用要如实记录。

已完成的代码级事实：

- 旧实现每个 `RedrawRequested` 都 `request_redraw()`（立即下一帧）。
- 新实现按 30 FPS 期限 `request_redraw_at(deadline)`；迟到事件不补画积压帧；
  暂停不续订、恢复冻结相位。单元测试覆盖排程、迟到、暂停/恢复。
- 本次**没有**采集 CPU/帧时间前后数据，因此不宣称性能收益。

## 7. 未执行 / 剩余限制

- 真实窗口的多浮窗菜单切换、关闭重开、迟到选择人工验证（代码路径已迁移）。
- 原生 WebView 矩形与面板头边界的视觉检查。
- 动画 CPU、重绘数的真实采样。
- Digger 最终 locked 全量构建：依赖图已无重复，待 ByteUI 新 tag 发布后验证。
- 候选 workflow 的远端实际运行（本地无法触发 GitHub Actions）。
- ByteUI 版本未 bump、未打包、未发布。

## 8. 本次实际执行的命令与结果

| 仓库 | 命令 | 结果 |
| --- | --- | --- |
| ByteUI | `cargo fmt --check` | 通过 |
| ByteUI | `cargo clippy --locked --all-targets -- -D warnings` | 通过（仅依赖 `block` 的 future-incompat 提示） |
| ByteUI | `cargo test --locked` | 130 passed，0 failed；1 个 ignored doc-test |
| Dozer | `cargo check -p dozer-app --tests` | 通过（既有 warning） |
| Dozer | `cargo test -p dozer-app theme::region` | 16 passed（含深浅方案与 ByteUI 的逐项对照、缩放值不二次缩放） |
| Dozer | `cargo test -p dozer-app menu` | 28 passed |
| Dozer | `cargo test -p dozer-app chrome::homespace` | 11 passed |
| Dozer | `cargo metadata --locked`（隐藏本地 patch） | 退出码 0 |
| Dozer | `scripts/check-shared-deps.sh --offline` | 0 阻断，0 警告 |
| Digger | `cargo check -p digger-desktop --tests`（临时 byteui patch） | 通过 |
| Digger | `cargo test -p digger-desktop native_menu`（临时 byteui patch） | 通过 |
| Digger | `cargo test -p digger-desktop settings::theme`（临时 byteui patch） | 编译/测试二进制通过 |
| Digger | 升级 `dozer-*` 到 `libs-2026-10-30a` 后隐藏本地 patch，`cargo metadata --locked` | 退出码 0；`bytehost-*` 无双版本 |
| Digger | `scripts/check-shared-deps.sh --offline`（新锁文件） | 没有发现问题 |
| Digger | `cargo fmt --all --check` | 通过 |

未执行：两产品 locked workspace/all-targets 完整构建、候选 workflow 远端运行、
真实窗口交互、动画性能采样、发布 tag/推送。


## 9. 审核修复（2026-10-31）

Dozer 主窗/浮窗已在当前工作区接入窗口级动画暂停。ByteUI 与 Digger 在本会话文件系统中
只读，因此修复在 `/private/tmp/byteui-review-fixes` 副本实施；可应用补丁与步骤见
[审核修复补丁](../patches/2026-10-31-byteui-review-fixes/README.md)。两个补丁针对当前开发结果，
不是针对 HEAD，均已对原工作区通过 `git apply --check`；随后用户已应用，反向检查通过且文件与验证副本逐字节一致。

- 修复提前重绘将期限推迟的问题，增加连续五次提前重绘回归测试。
- 测试恢复 guard 在 unwind 时还原颜色/revision、字体、几何与缩放的完整入口状态。
- 候选 workflow 从各自仓库更新临时锁后生成 tree/metadata，失败阻断；两消费方 ref 必填。
- 两产品窗口构建作用域传递暂停状态，避免逐个修改共享面板接口；Digger 焦点状态带窗口身份，
  与最小化状态分离。
- ByteUI 副本：133 项 lib 测试通过；fmt/clippy all-targets deny warnings 通过。
- 两产品副本：`cargo check --offline -p dozer-app --tests` / `cargo check --offline -p digger-desktop --tests`
  通过（副本临时锁解析，使用候选 ByteUI，不是发布组合验证）。
- Digger 副本：`cargo test --locked -p digger-desktop --lib product::tests::review_lifecycle` 22 项通过。
- CI shell：模拟 cargo 实际执行成功/失败路径，确认读取仓库配置、临时锁顺序、生成两侧解析图，
  tree 返回非零时任务失败且锁文件恢复；YAML 和 bash 语法检查通过。

真实菜单交互、动画耗电数据、远端 Actions 和正式 tag 升级仍未验收，不因本轮单测通过而标记完成。
Dozer 新调用依赖 ByteUI 补丁，合入/发布时必须配套，不能单独交付 Dozer 变更。
