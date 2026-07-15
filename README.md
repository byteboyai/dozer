# ByteBoy

面向 macOS（优先 Apple Silicon）的 AI 开发环境管理器。CLI 二进制名为 **`boy`**。

ByteBoy v0 本身**不**实现 agent、workflow 或任何 AI 逻辑，只负责管理本地 AI 开发环境：启动外部 AI CLI、在后台运行本地 MLX 模型、检查所依赖的工具是否安装。

## 功能

- **Agent 管理** —— 列出并启动外部 AI CLI（`claude`、`codex`、`hermes`、`aider`）。启动 agent 只是 exec 对应的 CLI，并透传额外参数。
- **MLX 模型管理** —— 对本地 `mlx_lm.server` 进程执行 start/stop/restart/status/logs，在后台运行。
- **Doctor** —— 检查所需工具链是否安装（Rust、Python、Git、uv、MLX、ComfyUI，以及你配置的 agent CLI）。
- **Config** —— 读取和编辑一份简单的 TOML 配置。

## 安装

需要 Rust 工具链（2024 edition），以及 Apple Silicon 的 macOS。

```bash
cargo build --release
# 二进制名为 `boy`
./target/release/boy --help
```

开发时可直接通过 Cargo 运行：

```bash
cargo run -- agent list
```

## 使用

```
boy run <agent> [args...]      启动一个 agent CLI（透传参数）

boy agent list                 列出已配置的 agent
boy agent doctor               检查 agent CLI 是否安装

boy model list                 列出已配置的模型
boy model start <id>           在后台启动模型
boy model stop <id>            停止运行中的模型
boy model restart <id>         重启模型
boy model status               查看所有模型状态
boy model logs <id>            查看模型日志

boy doctor                     检查开发环境
boy config show                打印当前配置
boy config edit                在编辑器中打开配置
boy version                    打印版本号
```

示例：

```bash
boy run claude --help          # 以 --help 调用 claude CLI
boy model start qwen36         # 为 qwen36 模型启动 mlx_lm.server
boy doctor                     # 查看哪些工具已安装、哪些缺失
```

## Shell 自动补全

ByteBoy 提供**动态补全**：补全时会读取你的 `config.toml`，因此 `boy run <TAB>`
会列出真实的 agent 名称，`boy model start <TAB>` 会列出真实的 model id（并以模型
显示名作为提示）。

启用方式（把对应行加入你的 shell 配置）：

```bash
# bash —— 加入 ~/.bashrc
source <(COMPLETE=bash boy)

# zsh —— 加入 ~/.zshrc
source <(COMPLETE=zsh boy)

# fish —— 加入 ~/.config/fish/config.fish
COMPLETE=fish boy | source

# elvish
eval (COMPLETE=elvish boy | slurp)
```

补全是运行时的：每次按 `<TAB>` 会执行一次 `boy` 来生成候选项，因此修改配置后
无需重新生成脚本即可生效。

## 配置

配置文件位于 `~/.config/byteboy/config.toml`。首次需要时会从默认模板自动创建。
后台模型的运行状态（PID 与日志文件）保存在 `~/.local/state/byteboy/` 下。

```toml
[agents]
claude = "claude"
codex = "codex"
hermes = "hermes"
aider = "aider"

[models.qwen36]
name = "Qwen3.6"
command = "mlx_lm.server"
model = "/Users/you/models/Qwen3.6"
host = "127.0.0.1"
port = 7101
```

- `[agents]` 将 agent 名称映射到用于启动它的 CLI 命令。
- `[models.<id>]` 定义一个模型。`boy model start <id>` 会把该条目翻译成后台运行的
  `mlx_lm.server --model <model> --host <host> --port <port>`；`stop`/`status` 据此查找进程。

## 开发

```bash
cargo build                 # 构建
cargo run -- <args>         # 运行（如 cargo run -- agent list）
cargo test                  # 运行全部测试
cargo clippy --all-targets  # lint
cargo fmt                   # 格式化
```

## 范围

v0 刻意保持最小。以下不在 v0 范围内（推迟到 v1+）：Skill、Workflow、Prompt、MCP、
Memory、Provider、RAG、任何 LLM API 调用，以及多模型路由。

完整设计文档（中文）是架构的权威来源：`docs/my/ByteBoy v0 开发文档.md`。
