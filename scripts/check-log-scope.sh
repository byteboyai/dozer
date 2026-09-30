#!/usr/bin/env bash
# 门禁:dozerd 与 dozer-app 里不允许裸 tracing 宏或 eprintln!——必须走
# dozer_core::log_*!(LOG, ...),日志才带来源(面板名/模块名)。
#
# - 给命令行用户看的 CLI 输出(如 dozerd 的参数错误提示,发生在日志初始化之前)可以在
#   行尾加 `// cli-output` 标记放行。
# - dozer-hook / dozer-mcp 不在此检查:它们的 eprintln! 多为 install/uninstall/launch
#   给命令行用户看的结果,运行期诊断用 dozer_core::plain_*!(由评审保证)。
# - 不用 clippy disallowed_macros:它会把 log_warn! 等包装宏的每个调用点都报成违规
#   (见 feat(core) tracing 后端提交里的 spike B),所以门禁只靠这个脚本。
set -euo pipefail
cd "$(dirname "$0")/.."
pattern='tracing::(error|warn|info|debug|trace)!|eprintln!'
hits=$(grep -rEn --include='*.rs' "$pattern" crates/dozerd/src crates/dozer-app/src | grep -v '// cli-output' || true)
if [ -n "$hits" ]; then
  echo "发现未带来源的日志调用(请改用 dozer_core::log_*!(LOG, ...);CLI 输出请加 // cli-output):" >&2
  echo "$hits" >&2
  exit 1
fi
echo "log scope check: ok"
