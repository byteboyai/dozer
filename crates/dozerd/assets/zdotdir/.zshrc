# Dozer shell integration —— .zshrc 阶段：转发用户原 .zshrc 后挂 OSC 7/133 发射钩子。
if [[ -n "$DOZER_ORIG_ZDOTDIR" ]]; then
  ZDOTDIR="$DOZER_ORIG_ZDOTDIR"
else
  unset ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshrc" ]] && builtin source "${ZDOTDIR:-$HOME}/.zshrc"

autoload -Uz add-zsh-hook
__dozer_osc7() { builtin printf '\e]7;file://%s%s\e\\' "${HOST:-localhost}" "$PWD" }
__dozer_preexec() { builtin printf '\e]133;C\e\\' }
__dozer_precmd() {
  local code=$status
  builtin printf '\e]133;D;%s\e\\' "$code"
  __dozer_osc7
}
add-zsh-hook preexec __dozer_preexec
add-zsh-hook precmd __dozer_precmd
__dozer_osc7
