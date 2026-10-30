# 共享设置抽取:两个 bytehost crate 的边界与本地 patch 现状

日期:2026-10-29(补记 2026-10-30)。

被记录对象:bytehost `crates/bytehost-settings`、`crates/bytehost-settings-ui`(均为 `0.1.0`,`publish = false`,已提交在 bytehost 分支 `feat/shared-settings` = `bba4ae8`,**尚未合进 main、尚未打 tag、尚未推送**),以及 Dozer 侧把设置窗口委托给这两个 crate 的改动。

Dozer 的版本号**没有动**(仍是 `0.5.307` / build `337`)。本仓惯例是版本号单独走一个 `chore(release)` 提交;而 `0.5.308` 现在根本发不出去(依赖声明的 tag 里没有这两个 crate),所以留到切 tag 那次一起推。

这篇存在的理由:`.cargo/config.toml`(gitignore,不进版本库)里有一段把 bytehost 全量 patch 到本地工作树的配置,注释指向本文。**去掉那段 patch 现在编不过**,原因见「本地 patch 现状」。

## 抽走了什么

| crate | 内容 | 行数 | 测试 |
|---|---|---|---|
| `bytehost-settings` | `page.rs`(PageId / PageDescriptor / PageKind / 保留命名空间校验)、`registry.rs`(有序页面注册表 + 重复/命名空间校验)、`state.rs`(打开、切页、关窗、窗口代数)、`selection.rs`(语言这类「选中 → 保存 → 确认 / 失败 / 冲突」通用模型) | 880 | 22 |
| `bytehost-settings-ui` | `shell.rs`(容器:标题 / 左侧导航 / 关闭 / 滚动)、`theme.rs`(主题页)、`language.rs`(语言页)、`section.rs`(heading / dim / warning / action_button / actions_row / separator / scrollable_body)、`text.rs`(文案接缝)、`apps/`(应用管理页:mod + rows + flow) | 1586 | 7 |

`bytehost-settings` 的固定页面 ID:`THEME_ID = "bytehost.theme"`、`LANGUAGE_ID = "bytehost.language"`、`APPS_ID = "bytehost.apps"`。

`bytehost-settings-ui` 另带一个 `examples/composition.rs`(110 行),演示产品怎么拼装这两层。

## 边界:刻意没抽的部分

- `bytehost-settings` 只依赖 `serde`(dev 加 `serde_json`)。**不**依赖 iced / byteui / wry / 任何 `dozer-*`、`digger-*` crate。
- `bytehost-settings-ui` 只产生 `Message` 和 `Element`。**不**创建窗口、不访问文件路径、不启动任务、不读写 Keychain。它依赖 `bytehost-apps`、`bytehost-panel`(应用页要 `InstallFlowState`)、`byteui`、`iced`(`default-features = false, features = ["tokio"]`)——渲染器后端由消费方决定,feature 合并后 `iced_renderer::Renderer` 是同一个类型。
- 留在 Dozer 的:窗口驱动与生命周期、服务调用、偏好落盘、主题实际应用、Git 账户页(`dozer.git`)、高级页(`dozer.advanced`,停/启 dozerd)、安装执行链。

## 两种扩展方式

1. **完整自定义页**:产品注册带自己命名空间的 `PageId`(如 `dozer.git`、`dozer.advanced`),在容器 `content` 里直接放自己的 `Element<M>`。`bytehost.*` 是保留命名空间,产品页不能用。
2. **内置页附加区块**:每个内置页接受 `extra: Vec<Element<M>>`,追加在共享内容之后。应用页更细,分 `extra_top` / `extra_bottom`,外加 `app_notes`、`app_leading_actions` 两个按应用行注入的钩子。

Dozer 当前两种都用了:主题页用 `extra` 挂「保存失败」提示;Git / 高级页是完整自定义页。

## 文案接缝(容易踩)

`bytehost-settings-ui` **不导入任何产品的 i18n crate**。它接受注入的 `Text`,签名是 `(key, english_fallback) -> String`,文本空间 owner 是 `Text::OWNER = "bytehost.settings"`。没有翻译时**必须**返回英文 fallback。

Dozer 侧接法(`crates/dozer-app/src/extensions/settings.rs`):`shared_text(key, fallback)` 转 `crate::i18n::host_text(key, fallback, locale)`;locale 存在 thread_local `SETTINGS_LOCALE` 里,渲染前用 `with_locale(locale, || …)` 设好、渲染完恢复。

**坑:** `text.rs` 里的内置英文回退表 `english_fallback()` 只覆盖 14 个 key(`settings.title` / `settings.close` / `settings.empty` / `settings.theme.*` / `settings.language.*`),**应用页的 key 不在表里**,靠调用点就地写的英文 fallback。也就是说产品词表必须自己把所有用到的 key 补齐,漏一个就退化成英文。

本次就漏了 4 个,已修:

| key | 情况 |
|---|---|
| `settings.theme.title` | en / zh 都缺 → 中文界面主题页标题显示英文。已补。 |
| `settings.apps.probe_failed_label` | 共享层把 `{why}` 拼在标签外面(`format!("{}: {why}", …)`),Dozer 旧词表里是 `settings.apps.probe_failed`(带 `{why}`),对不上。已改名为 `_label` 并去掉 `{why}`。 |
| `settings.apps.read_failed_label` | 同上。 |
| `settings.empty` | 故意不译:Dozer 的注册表是固定 5 页 + `.expect()`,这条到不了。 |

另外新增一个 key:`settings.theme.save_failed`(带 `{reason}`),对应主题落盘失败时挂在主题页 `extra` 里的红字——原来是硬编码英文,现在走 `host_text_args`。

en / zh 两份词表现在各 299 个 key,键集完全一致。

## Dozer 侧改动清单

| 文件 | 改动 |
|---|---|
| `crates/dozer-app/Cargo.toml` | 加 `bytehost-settings` / `bytehost-settings-ui` 依赖(声明 tag `v0.4.17`,见下) |
| `src/extensions/settings.rs` | 页面身份/注册表/窗口状态/语言保存状态改走 `bytehost_settings`;容器、主题页、语言页改走 `bytehost_settings_ui`;新增 `persist_theme() -> Result<(), String>` 与 `State.theme_error` |
| `src/extensions/settings_apps_view.rs` | 1044 行 → 33 行,变成对 `bytehost_settings_ui::apps::view` 的薄委托 |
| `src/app/app.rs` | 适配 `SelectionState` API:`state.language.update_confirmed(…)`、`!s.language.is_saving()` 取代直接读字段 |
| `src/extensions/app_logs.rs` | 原来的实现断言改成源码哨兵(断言 `settings_apps_view.rs` 里出现 `bytehost_settings_ui::apps::view`) |
| `locales/host/{en,zh-CN}.json` | 上一节那 4 + 1 个 key |

`Cargo.lock` 与 `packaging/macos/Info.plist` **不在提交里**,原因见「本地 patch 现状」第 4、5 条。

绿灯状态(2026-10-30 复跑):Dozer `cargo fmt --check` 干净、`cargo clippy --all-targets` exit 0(本次改到的 5 个文件一条 warning 都没有)、`cargo test -p dozer-app` 1133 passed / 0 failed(其中 settings 相关 47 条、`app_logs` 哨兵 2 条)。bytehost 侧 `cargo fmt --check`、`cargo clippy -p bytehost-settings -p bytehost-settings-ui --all-targets --all-features -- -D warnings`、`cargo test`(22 + 7)、`cargo doc --no-deps`、`scripts/check-deps.sh` 全绿。

提交前 clippy 拦下两处、已修:

- `bytehost-settings/src/lib.rs` 的模块列表里 `` [`state`]:… `` 因为冒号后没有空格,被 markdown 当成链接引用定义(`clippy::doc_nested_refdefs`)。四条统一写成 `` [`x`][]:… ``。
- `bytehost-settings-ui/src/apps/` 的 `app_notes` / `app_leading_actions` 类型触发 `clippy::type_complexity`,抽出公开别名 `AppRowExtra<'a, M> = dyn Fn(&AppSummary) -> Vec<Element<'a, M>> + 'a`。别名是透明的,Dozer 侧传闭包的写法不用改。顺带删掉 `rows.rs` 里 `Flow::ConfirmRollback { id, to: _, .. }` 的多余 `to: _`。

## 本地 patch 现状(重要)

1. `.cargo/config.toml`(gitignore)把 9 个 bytehost crate patch 到 `../bytehost/crates/*`,同时还 patch 了 `byteui` → `../byteui`、`bytegit` → `../bytegit`。
2. **`crates/dozer-app/Cargo.toml` 给两个新 crate 写的 tag 是 `v0.4.17`,而 `v0.4.17` 里没有这两个 crate。** 现在能编纯粹是靠上面那段 patch 盖过去了。别人克隆 Dozer、没有这份 gitignore 的 config,`cargo build` 会直接报找不到包。
3. bytehost 侧:已从 `main`(`87f1c28`,跟踪 `origin/main`)另开分支 `feat/shared-settings`,提交两个 crate 与 `Cargo.lock` 里那两条新 package 条目(`bba4ae8`),与 `feat/web-find` 那 3 个页内查找提交完全分开。**两个分支都未推送、无 upstream。**
4. Dozer 的 `Cargo.lock` **不能提交**。patch 让它丢了 8 行 `source = "git+…"`(6 个 bytehost + byteui + bytegit)、多出 `[[patch.unused]] bytehost-chrome`(patch 了但没人依赖),还把 `windows` 0.58.0 → 0.57.0、`windows-sys` 0.61.2 → 0.60.2 降了档(本地 byteui / bytegit 工作树与钉住的 tag 解析结果不同)。提交它等于把三个共享库从 tag 上解开,坑到所有没有这份 gitignore config 的人。所以本次提交跳过 `Cargo.lock`,让它留在工作树里当本地状态,切回 tag 后重新生成。
5. bytehost 的 `Cargo.lock` 相反,**可以**提交、也已经提交:那个仓没有 patch,lock 的 diff 只有两条新 package 条目。同名文件在两个仓里处置相反,别搞混。
6. `packaging/macos/Info.plist` 与 `dozer-app` 的 `version` 也没进提交——见开头那段。

## 切回发布 tag 的步骤

1. ~~bytehost:从 `main` 开分支提交两个 crate~~ **已完成**:`feat/shared-settings` = `bba4ae8`(17 个源文件 + `Cargo.lock` 两条新条目)。剩下的是推送、合进 `main`。**推送要先问。**
2. bytehost:打新 tag(下一个是 `v0.4.18`)并推送。**这步和推送都要先问。**
3. Dozer:`crates/dozer-app/Cargo.toml` 里**所有** bytehost 依赖的 `tag = "v0.4.17"` 一起改成 `v0.4.18`(不要只改新加的两个,同一个 git 源混用两个 tag 会让 cargo 拉两份)。
4. Dozer:删掉 `.cargo/config.toml` 里整段 `[patch."https://github.com/byteboyai/bytehost"]`。注意同一 git 源做 `[patch]` 会被 cargo 判为「同一 source」而报错,所以只能整段删,不能留一半。`byteui` / `bytegit` 的 patch 按需保留。
5. Dozer:`cargo update -p bytehost-settings -p bytehost-settings-ui`(或直接 `cargo check` 让它重解析),确认 `Cargo.lock` 里 8 行 `source = "git+…"` 回来了、`[[patch.unused]] bytehost-chrome` 消失了、`windows` / `windows-sys` 回到 tag 的解析结果。
6. Dozer:这一次 `Cargo.lock` **要**提交——此时它才是干净的(只有两个新 package 条目加上 dozer-app 自己的依赖变化)。
7. Dozer:`cargo check -p dozer-app && cargo test -p dozer-app settings`。
8. Dozer:版本推一档,照 `c26dd4ca`(`chore(release): 0.5.307`)的样子单独一个提交——`crates/dozer-app/Cargo.toml` 的 `version`、`Info.plist` 的 `CFBundleVersion` 与 `CFBundleShortVersionString`、`Cargo.lock` 里 dozer-app 的 version 行。

## 待办(不属于本次)

- **`bytehost/scripts/check-deps.sh` 没有覆盖这两个新 crate。** 它的分组是硬编码的(`bytehost-apps` / `-client` / `-webview` / `-panel`、`bytehost-core`、`bytehost-desktop`),两个 settings crate 不在任何一组里。也就是说那句 `bytehost deps check: ok` **并没有**验证 `bytehost-settings` 自己文档里写的「不依赖 iced / byteui / wry」;只有 pins 组(byteui / bytegit 钉版一致性)通过 `crates/*/Cargo.toml` 通配扫到了它们。要把承诺变成门禁:`bytehost-settings` 加进「产品 + 框架都禁」那组、`bytehost-settings-ui` 加进「只禁产品、允许 iced/byteui」那组。**没动,等决定。**
- GUI 人工验收还没做:S1–S16 副本收在 `docs/superpowers/manual_tests/2026-10-30-pending-manual-acceptance.md` §1(结果填在那里);出处与机械项 M1–M13 见 `docs/superpowers/reviews/2026-10-30-shared-settings-acceptance.md`。
- ~~`crates/dozer-app/web/review-trace/` 只有 `node_modules`、没有任何源文件、全仓无人引用 —— 垃圾目录,可删(要问)。~~ **已删(2026-10-30,经用户同意)。** 更正一句:旧文档史里大量出现这个路径(review-trace 源码 2026-10-26 S7a 起从 `dozer-app/web/` `git mv` 到 `dozer-panel-conversations/web/`,被搬走的只有跟踪文件,这份 `node_modules` 是搬家后的遗留),但活代码与构建脚本只指新路径。顺手把 `.qoder/settings.local.json` 加进了 `.gitignore`。
- 浏览器迁移计划的 Task 4 人工验收(D1–D18,副本在汇总 §2)、以及 ⌘F 页内查找要不要做,都还在等用户决定。
