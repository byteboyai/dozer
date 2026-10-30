# 共享设置抽取:设置窗口人工验收清单

日期:2026-10-30。

> **下面 S1–S16 的副本已收进 `docs/superpowers/manual_tests/2026-10-30-pending-manual-acceptance.md` §1,结果填在那里**(那份汇总把设置、浏览器、应用宿主三条线的待验项排成一次能走完的顺序)。本文保留被验收版本、机械项 M1–M13 与「如果发现问题」的定位指引,是这次改动的完整记录。

被验收:Dozer 的设置窗口委托改动(提交 `653034ef`;版本号仍是 `0.5.307` / build `337`,理由见笔记)+ bytehost 两个**尚未发布**的 crate(`bytehost-settings` / `bytehost-settings-ui`,分支 `feat/shared-settings` = `bba4ae8`,靠 `.cargo/config.toml` 的本地 patch 生效)。抽取的边界与现状见 `docs/superpowers/notes/2026-10-29-shared-settings-inventory.md`。

**我做不了这次验收。** 两条都堵住了:

1. 当前模型读不了图片,截图对我是不可见的;
2. Orca 的合成点击 / 键盘进不了 iced 控件(与 `2026-10-28-dozer-browser-acceptance.md` 里记的限制相同),设置窗口又只能从顶栏齿轮点开(`Message::SettingsOpen` 只有 `chrome/topbar.rs` 一个产生点,没有 ⌘, 快捷键),盲点坐标无法自证结果。

另外设置这条链**一行日志都没有**(`extensions/settings.rs`、`platform/settings_overlay.rs` 里没有任何 `log_*!`),所以也没法用日志侧面确认「页面切过去了 / 保存成功了」。下面 5 页全部要你来点。

## 已验(我做的,机械项)

| # | 项 | 结果 |
|---|----|------|
| M1 | `cargo check -p dozer-app` / `cargo build -p dozer-app` | ✅ exit 0(21 条既有 warning,与本次无关) |
| M2 | `bytehost-settings` 单测 | ✅ 22 passed |
| M3 | `bytehost-settings-ui` 单测 | ✅ 7 passed |
| M4 | `cargo test -p dozer-app settings` | ✅ 47 passed |
| M5 | 两仓 `cargo fmt --check` | ✅ 干净 |
| M6 | `bash scripts/check-i18n.sh` | ✅ 绿(16 个 owner 目录;inventory total=2781 unresolved=0;357 个 key 引用都有资源;检查器自测 27 passed) |
| M7 | en / zh-CN 两份 host 词表键集 | ✅ 各 299 key,`en-zh` 与 `zh-en` 差集均为空 |
| M8 | 隔离 `HOME` 启动 dev 构建 | ✅ 起来干净,自动拉起 dev `dozerd 0.3.3`,socket 落在沙箱内(`/tmp/…/home/Library/Application Support/ai.byteboy.dozer/dozerd.sock`),无 panic;**没有碰你正在用的 Dozer 数据** |
| M9 | `cargo clippy --all-targets`(Dozer 全 workspace) | ✅ exit 0;本次改到的 5 个文件一条 warning 都没有(41 条既有 warning 全在别处) |
| M10 | `cargo test -p dozer-app` 全量 | ✅ 1133 passed / 0 failed / 2 ignored |
| M11 | bytehost `cargo clippy -p bytehost-settings -p bytehost-settings-ui --all-targets --all-features -- -D warnings` | ✅ 绿(提交前拦下 2 类问题,已修,见笔记「Dozer 侧改动清单」后面那段) |
| M12 | bytehost `cargo doc --no-deps`(两个新 crate) | ✅ 无 warning,intra-doc 链接都解析得到 |
| M13 | bytehost `bash scripts/check-deps.sh` | ✅ `bytehost deps check: ok`——但**注意它没覆盖这两个新 crate**,见笔记「待办」 |

M8 有个附带结论:第一次我用后台任务方式启动,任务包装被拆掉时 GUI 跟着退出了(exit 0,日志无异常);改用 `nohup … & disown` 后进程稳定存活。**不是应用缺陷**,是启动方式的问题。

## 怎么跑

```bash
cargo build -p dozer-app -p dozerd          # dozerd 要一起编:GUI 优先用同目录的 dozerd
V=/tmp/dozer-settings-verify; mkdir -p "$V/home"
nohup env HOME="$V/home" RUST_LOG=info \
  /Users/chrischiang/Projects/CoralProjects/byteboy/target/aarch64-apple-darwin/debug/dozer \
  > "$V/dozer.log" 2>&1 & disown
```

数据目录跟着 `HOME` 走,所以这样起的是一个全新的空实例,和你正在用的那个(`/Applications/Dozer AI Coder.app`)互不干扰。收摊:

```bash
pkill -f 'aarch64-apple-darwin/debug/dozer'; rm -rf /tmp/dozer-settings-verify
```

**语言:** 验收 zh 的项之前,先在「语言」页把界面切成简体中文(全新沙箱默认跟随系统)。

## 要你验的

| # | 怎么做 | 期望 | 结果 | 备注 |
|---|--------|------|------|------|
| S1 | 点顶栏齿轮打开设置 | 窗口打开,左侧 5 项按序:**主题 / Git 账户 / 高级 / 应用 / 语言**,默认停在「主题」 | | 顺序由 `product_registry()` 决定,有单测兜底 |
| S2 | 中文界面下看「主题」页标题 | 显示 **「主题」**,不是 `Theme` | | **本次修的回归**:共享层用 `settings.theme.title`,Dozer 词表原先缺这个 key |
| S3 | 主题页两个选项 | 「深色 · ByteBoy2077」「浅色 · ByteBoy2077-Light」,当前项有 ✓ 且是金色 | | |
| S4 | 点另一个主题 | 整个应用配色立刻变;重开设置后选择还在 | | 落盘走新的 `persist_theme()` |
| S5 | 让主题落盘失败一次(比如把偏好文件设成只读)再切主题 | 主题页底部出现红字:**「主题保存失败(…)。重启前仍然生效,可再选一次重试。」** | | **本次修的硬编码英文**;括号里是原因。构造不出来就跳过 |
| S6 | 切到「Git 账户」页 | 三行 GitHub / GitLab / Gitee;未连接时显示「未连接」+「连接」 | | 完整自定义页,没走共享视图 |
| S7 | 切到「高级」页 | 显示「停止 dozerd」,下面是停止提示文案 | | 全新沙箱里 dozerd 是 dev 实例,停了会影响后续项,建议放在最后验 |
| S8 | 切到「应用」页 | 顶部「运行时(受管版本随 dozerd 安装在本机,优先于系统版本)」;下面是探测结果;再下面「已安装」 | | 这页自带滚动,容器不再包一层(`scroll_content: page != APPS_ID`) |
| S9 | 「应用」页在 daemon 不可用时(dozerd 停掉后重开设置) | 显示 **「探测失败: …」** / **「读取失败: …」**,中文标签 + 冒号 + 原因 | | **本次修的 key 改名**:`probe_failed{why}` → `probe_failed_label`(冒号由共享层拼)。这是最容易看出问题的一项 |
| S10 | 「应用」页点某个已安装应用的「日志」 | 日志展开,内容持续输出时**自动滚到底** | | 滚动 id 由 Dozer 的 `app_logs::scroll_id` 注入共享视图 |
| S11 | 「应用」页走一次安装(本机目录 / 压缩包 / URL 任一) | 审批卡出现:来源、将要做的事、申请的权限;批准后进度与结果正常 | | 安装执行链留在 Dozer,共享层只出视图 |
| S12 | 切到「语言」页,选另一个语言 | 选项立刻显示「保存中…」,成功后整个界面文案换语言,已选项打 ✓ | | `SelectionState` 的乐观保存 |
| S13 | 「语言」页:保存期间在别处改语言(或让保存失败) | 出现「语言已在别处被修改,请重新选择。」/「语言设置保存失败」 | | `update_confirmed` / `is_saving` 是本次改的 API 接法 |
| S14 | 左侧 5 项来回点、鼠标悬停关闭按钮、按 Esc / 点关闭 | 切页流畅无残影;关闭按钮有悬停态;窗口能关掉再打开,重开后停在上次那页或回到「主题」(以现状为准,只要不崩) | | 容器是共享的 `settings_shell` |
| S15 | 窗口很窄 / 很矮时看 5 页 | 不溢出、不截断关键控件;「应用」页能滚 | | |
| S16 | 英文界面下把 S2–S13 再扫一遍 | 全英文,没有中英混排 | | 尤其 S9 的两个失败标签 |

## 如果发现问题

- 文案类(缺 key、占位符没替换、中英混排):改 `crates/dozer-app/locales/host/{en,zh-CN}.json`,然后 `bash scripts/check-i18n.sh` 必须绿。对共享层 key 清单:

  ```bash
  rg -o 'settings\.[a-z_.]+' ../bytehost/crates/bytehost-settings-ui/src -N --no-filename | sort -u
  ```

  当前跑出 **63 个 key**,其中只有 `settings.empty` 不在 Dozer 词表里——那是故意的(Dozer 的注册表是固定 5 页,这条到不了)。多出来的就是漏译。

- 布局 / 交互类:先看是共享视图(`../bytehost/crates/bytehost-settings-ui`)还是 Dozer 适配层(`crates/dozer-app/src/extensions/settings.rs`)的问题——共享层改了要重新在 bytehost 侧提交。
- 状态机类(保存中 / 冲突 / 确认):`bytehost-settings` 的 `selection.rs`,Dozer 侧 47 个测试大多覆盖这里。
