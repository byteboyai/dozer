# Dozer shell integration —— .zshenv 阶段：转发用户原 .zshenv 后回到包装目录。
# 已知边界：用户 .zshenv 若自改 ZDOTDIR，会被此处覆盖（spec P1e D2）。
if [[ -n "$DOZER_ORIG_ZDOTDIR" ]]; then
  ZDOTDIR="$DOZER_ORIG_ZDOTDIR"
else
  unset ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]] && builtin source "${ZDOTDIR:-$HOME}/.zshenv"
ZDOTDIR="$DOZER_ZDOTDIR_WRAPPER"
