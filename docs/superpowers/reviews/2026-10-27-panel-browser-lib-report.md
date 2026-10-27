# 浏览器面板抽库验证报告(2026-10-27)

验证对象:`feat/panel-browser-lib` @ 4a66c574。方法:`git archive HEAD` 导出到 `/private/tmp/bb-lib-snap`(干净快照,独立 `CARGO_TARGET_DIR`),不含工作区未跟踪文件。

## 结果

| 项 | 结果 |
|----|------|
| `cargo build --workspace` | 通过(仅 warning) |
| `cargo test --workspace --no-fail-fast` | 3576 passed / 0 failed / 5 ignored |
| `cargo clippy --workspace --all-targets` | 退出 0,64 条 warning,0 error(未与基线逐条比对) |
| `cargo fmt --check` | 通过 |
| `check-i18n.sh` / `check-log-scope.sh` / `check-shared-deps.sh` | 全部通过(shared-deps 有一条 guard-source 版本提示 INFO,非阻断) |
| `check_panel_boundary.py` | 通过(205 refs / 5 files,baseline ratchet) |
| `scripts/audit` python 单测 | 48 OK |
| `cargo tree -p dozer-panel-browser \| grep -E "dozer-(app\|client\|core)\|dozerd\|wry\|winit"` | 无输出 |

库依赖闭包(直接):bytehost-core、byteui、dozer-i18n、dozer-model、dozer-panel-host、dozer-services、iced_renderer、iced_widget、tokio、tracing。

## 测试数核对

- 基线:`extensions::browser` 41 条,workspace 3563。
- 终态:`dozer-panel-browser` 53 条(0 ignored),dozer-app 1130 passed / 2 ignored,workspace 3576。
- workspace 净增 13 = 库内 53 - 搬走 40。基线 41 条中有 1 条未能在库里按名对上(dozer-app 内已无 `extensions::browser` 前缀测试)。**此差 1 条未逐条核实**,需要对照基线日志 `cargo test -p dozer-panel-browser -- --list` 确认,本次验证未做。
- 无偶发失败,全部一次通过(未重跑)。

## 未覆盖

- 未做逐条名字对照与 Review Focus 五条到测试名的映射。
- 未发现真实 bug。
- 人工目测验收清单见 plan Task 6 Step 5,尚未进行。
