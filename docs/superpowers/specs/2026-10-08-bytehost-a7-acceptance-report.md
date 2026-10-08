# bytehost A7 验收报告(拆分为独立仓库 `byteboyai/bytehost`)

> 状态:**完成**。A7 计划(`docs/superpowers/plans/2026-10-08-bytehost-a7-extract-standalone-repo.md`)
> 的 Task 6(拆仓 + 独立构建 + CI 发布)已全绿并推送到 `byteboyai/bytehost`;Task 7(dozer 改依赖 tag、
> 删除重复)另行提交。
> 本报告对应 Task 6 Step 7,记录 Step 1–6 的真实输出与新仓库首个提交 id / tag。

## 0. 环境与对象

- 平台:macOS 26.6.2(Darwin arm64)。
- 运行时(本机):`python3` = 3.13.1(homebrew)、`node` = v24.14.0。
- 工具:`git-filter-repo`(homebrew `/opt/homebrew/bin/git-filter-repo`)、`gh`(已登录 `chrischiangs`)。
- 一次性克隆目录:`$TMPDIR/opencode/bytehost-extract`;独立构建验证目录:`$TMPDIR/opencode/bytehost-clean`。
  (计划文本里写的是 `/tmp/claude-501/...`;本会话的临时目录不同,语义一致——都是"dozer 之外的一次性克隆"。)
- **新仓库**:`https://github.com/byteboyai/bytehost`
  - `main` HEAD = tag `v0.1.0` = `20c013ba3262ff25309c58b9c40a2834d3cd31f2`
  - 首个发布提交(顶层文件补齐)= `9eb626ab9ab1217f950f0656848e809b897faa4d`
  - 历史提交数 `90`(filter-repo 保留原 `bytehost-apps` 历史;`git log --oneline -- crates/bytehost-apps` = 50)
  - 顶层路径:`.github/ .gitignore Cargo.lock Cargo.toml README.md crates/ docs/ scripts/`
- 四个 crate:`bytehost-apps`、`bytehost-client`、`bytehost-webview`、`bytehost-panel`。

## 1. 执行结果——已验证

### 1.1 Step 1:克隆 + `git filter-repo`(保历史)

命令(一次性克隆,不碰 dozer 仓库):

```
git clone --no-local . $TMPDIR/opencode/bytehost-extract
git filter-repo \
  --path crates/bytehost-apps --path crates/bytehost-client \
  --path crates/bytehost-webview --path crates/bytehost-panel \
  --path scripts/bytehost --path scripts/check-bytehost-apps-deps.sh \
  --path docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md \
  --path-glob 'docs/superpowers/specs/*bytehost-a*-acceptance-report.md' \
  --path-rename scripts/check-bytehost-apps-deps.sh:scripts/check-deps.sh \
  --path-rename scripts/bytehost/:scripts/
```

结果:`git log --oneline | wc -l` = 89(≥ 41 满足预期;`-- bytehost-apps` = 50);`git ls-files` 顶层只剩
`crates docs scripts`;`scripts/check-deps.sh`、`scripts/samples/*`、设计规格与 a5/a6e–a6h 验收报告都在。

**偏差 1:** 计划预期 `docs/` 里有 A0–A6 全套验收报告,但 dozer 仓库里实际只存在 `a5/a6e/a6f/a6g/a6h`
五份(`a0`–`a4` 从未落盘)。glob 忠实照搬了现存文件,没有凭空造不存在的报告。

### 1.2 Step 2:补顶层文件

新增(都在新仓库):

- `Cargo.toml`:`[workspace] resolver = "3" members = ["crates/*"]`,
  `[workspace.package] edition = "2024" license = "MIT"`;`[workspace.dependencies]` 抄 dozer 的
  `tokio/tracing/serde/serde_json/uuid/libc` 版本。**不依赖任何 dozer crate。** crate 之间用直接
  `path = "../..."`(与 dozer 同风格,不用 workspace 继承)。
- `README.md`:四个 crate 各一句话 + 消费者(Dozer 已接入 / Digger 嵌入待做)+ 兼容性承诺
  (wire 只追加、落盘格式带版本、tag 即发布、`data_store_identifier` 算法不改)+ 开发/测试命令。
- `.github/workflows/ci.yml`:runner `macos-latest`,步骤 `cargo fmt --all --check`、
  `cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、
  `bash scripts/check-deps.sh`(仿 byteui 的 CI,但按本仓库加了 `--all-features` 与门禁)。
- `scripts/check-deps.sh`:原 `check-bytehost-apps-deps.sh` 扩展成四条门禁(见 §1.5)。
- `.gitignore`:`/target`(filter-repo 产物不带 dozer 的 `.gitignore`,本机构建出 `target/` 必须忽略)。
- `crates/bytehost-client/examples/embed.rs`:30 行嵌入演示(见 §1.4)。
- `crates/bytehost-client/tests/embedded_host.rs`:端到端嵌入验证(见 §1.4)。

**路径修正:** 搬迁后 `scripts/bytehost/` → `scripts/`,源码里写死的旧路径全部改掉:

- `crates/bytehost-apps/src/manifest.rs`(`include_str!` + 文档示例 + 测试)
- `crates/bytehost-apps/tests/process_apps_live.rs`(2 处)
- `crates/bytehost-apps/src/runtime/managed/{pins.rs,mod.rs}`(文档注释)

`grep -rn "scripts/bytehost" crates` 在新仓库无结果。

### 1.3 Step 3:独立构建(无 dozer 在旁)

在**全新克隆**(`$TMPDIR/opencode/bytehost-clean`,`git clone --no-local` 上面结果;无 `.cargo/config.toml`
patch)里运行,全部通过:

| 命令 | 结果 |
|------|------|
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --all-targets --all-features -- -D warnings` | 通过(无警告) |
| `cargo test --all-features` | **408 passed, 0 failed**(+ `bytehost-panel` 95、`bytehost-webview` 13 等;8 ignored) |
| `bash scripts/check-deps.sh` | `bytehost deps check: ok` |
| `cargo test -p bytehost-apps --all-features --test process_apps_live -- --ignored` | **7 passed**(需要真 python3/node) |
| `cargo test -p bytehost-client --all-features --test embedded_host -- --ignored` | **1 passed** |

**偏差 2(CI 首跑失败 → 已修):** 首次 CI 在 `macos-latest` 上 **15 个用例失败**,全是"真起 python 服务器 +
健康探测"的用例。根因经 CI runner 上实测确认:`python3` = **3.14.7**,而 `http.server.HTTPServer.server_bind`
/ `ThreadingHTTPServer` 在启动时调用 `socket.getfqdn('127.0.0.1')`,在**没有反向 DNS 的 runner 上阻塞约 35 秒**
(diag 实测 `getfqdn('127.0.0.1') -> 'localhost' in 35.04s`),服务器在测试探测窗口内根本没到 `serve_forever`。
`socketserver.TCPServer`/`ThreadingTCPServer` 不调 `getfqdn`,同环境可秒级就绪(diag 实测 connect 成功)。
修法:**只改测试/样例的服务器片段**(5 处测试片段 + `scripts/samples/py-notes/server.py`),把
`http.server.test(...)` / `ThreadingHTTPServer(...)` 换成 `socketserver.ThreadingTCPServer(...).serve_forever()`,
语义不变(静态服务 + 自定义 handler)。修完 CI 转绿。**这不是 bytehost 逻辑 bug,是测试夹具对"启动期反向 DNS"
的隐式依赖;顺带修掉了 `py-notes` 样例在同一类主机上启动慢几十秒的问题。**

### 1.4 Step 4:嵌入验证(Digger 用法,没有 dozerd)

`crates/bytehost-client/tests/embedded_host.rs`:不依赖任何 dozer 代码,`AppService::start_with(root, GatewayConfig{port:0})`
+ `InProcess`,装并启动 `scripts/samples/py-notes`,经 gateway 断言:带 Cookie 请求得 **200** 且 body 含 `runtime:`;
不带 Cookie 得 **403**;`shutdown()` 后端口关闭。结果 **1 passed**。

> **偏差 3:** 计划把该测试放在 `crates/bytehost-apps/tests/`,实际放在 **`crates/bytehost-client/tests/`**——
> 因为它验证的是 `bytehost-client` 的 `InProcess` 端到端(装/启动/网关/shutdown 全走 `AppHostApi`),放 client
> 侧更贴职责。且该测试必须在 `#[tokio::test(flavor = "multi_thread")]` 下跑(测试体在客户端线程上做阻塞 socket
> I/O,current_thread 运行时会死锁),与 `process_apps_live` 同款。

`crates/bytehost-client/examples/embed.rs` 是同流程的 30 行演示,实跑输出:
`已启动 py-notes;首次导航地址:http://py-notes.localhost:<port>/?bh_token=…`(需绝对应用目录)。

### 1.5 Step 5:变异验证(门禁本身)

对 `crates/bytehost-panel`:

1. 在 `Cargo.toml` 追加 `iced_core = "0.13"` → `check-deps.sh` **失败**:
   `bytehost-panel 的依赖里出现了被禁的 crate: iced_core`(exit 1)。恢复后 ok。
2. 在 `src/lib.rs` 追加含 `AppSlot` 字样的注释 → **失败**:
   `crates/bytehost-panel/src/lib.rs:18:// AppSlot` / `bytehost-panel 源码里出现了 AppSlot/toast(应改用 AppKey/Notice):`(exit 1)。恢复后 ok。

两处变异均已还原,干净克隆 `git status` 为空。

门禁四条(全对 `cargo tree` / 源码执行):① 任一 crate 依赖树不得出现 `dozer*`、`iced*`、`wry`、`tauri*`、`objc2*`;
② `bytehost-apps` 默认 feature 依赖闭包只能是 serde 家族;③ `bytehost-panel` 的依赖只能是 `bytehost-apps`、
`bytehost-client` 及其传递依赖;④ `grep -rn "AppSlot\|toast" crates/bytehost-panel/src` 无结果。

### 1.6 Step 6:发布

用户明确授权后执行:

```
gh repo create / 确认空仓库 byteboyai/bytehost(此前已存在且为空)
git remote add origin https://github.com/byteboyai/bytehost.git
git branch -M main
git push -u origin main
git tag v0.1.0 && git push origin v0.1.0
```

首次推送后诊断过程中产生过一批临时 `diag` 提交;诊断完成后 **force-push 清掉**,只保留"独立仓库提交 + getfqdn 修复"
两个提交,并把 `v0.1.0` 移到含修复的提交上。最终 CI 全绿:

- CI run `37758759810`(commit `20c013b`)= **success**(2m11s):fmt / clippy `-D warnings` / `test --all-features` / check-deps。

## 2. 偏差汇总(相对计划文本)

1. 只存在 a5/a6e–a6h 五份验收报告(无 a0–a4),glob 照实搬迁。
2. CI 首跑因 Python 3.14 + runner 无反向 DNS 失败,已按上述方式修复并转绿。
3. `embedded_host.rs` 放在 `bytehost-client/tests/`,且需 `multi_thread` tokio runtime。
4. 一次性克隆/独立构建的临时目录与计划文本示例不同(语义一致)。
5. `.gitignore` 为额外新增(filter-repo 产物不自带,否则 `target/` 会被提交)。

## 3. 未做 / 交给 Task 7

- dozer 侧尚未切到 tag、尚未删除 in-tree 的四个 crate 与 `scripts/bytehost/`、`scripts/check-bytehost-apps-deps.sh`、
  尚未更新 `CLAUDE.md` / `byteboy-repositories.md`——这些是 **Task 7** 的范围,另行提交。
- 新仓库的发布节奏/changelog 自动校验、Tauri 侧 webview 创建代码、Digger 自带守护进程,均不在本切片。
