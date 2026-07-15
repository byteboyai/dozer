# ByteBoy v0 设计方案

> Date: 2026-06-23
> Status: Approved
> Source: `docs/my/ByteBoy v0 开发文档.md`

## 1. 目标与范围

ByteBoy 是一个 **AI 开发环境管理 CLI**（binary 名 `boy`），面向 macOS（Apple Silicon 优先）。v0 **不实现** Agent 逻辑、Workflow、Skill、MCP、任何 LLM API 调用，只做本地 AI 开发环境的统一管理：

- Agent 管理：列出 / 启动外部 AI CLI（claude、codex、hermes、aider），检查是否安装。
- MLX 模型管理：后台启动 / 停止 / 重启 / 查看状态 / 查看日志 `mlx_lm.server` 进程。
- Doctor：检查开发环境所需工具是否安装。
- Config：查看 / 编辑 TOML 配置。
- Version。

本次开发覆盖**完整 v0**（上述全部命令）。

## 2. 工程结构

**单 crate** `byteboy`，binary 名通过 `[[bin]] name = "boy"` 指定。Rust 2024 edition。模块划分：

```
src/
  main.rs        // 入口：初始化日志、解析 CLI、分发、统一错误输出
  cli.rs         // clap derive 命令定义
  context.rs     // Context（持有解析后的 Config 与路径）
  error.rs       // thiserror 领域错误
  config.rs      // Config / Agents / Model 的 serde 结构体 + 加载逻辑
  paths.rs       // 配置目录、状态目录解析
  agent.rs       // agent list / run / doctor
  model.rs       // model list / start / stop / restart / status / logs
  doctor.rs      // 环境检查
  process.rs     // PID 文件读写、进程存活校验、PATH 中查找命令
```

> 文档第 3 节描述的多 crate workspace 推迟到逻辑变重时再拆分（遵循“保持简单，避免过度设计”）。

### 依赖

- `clap`（derive feature）— CLI 解析
- `serde` + `toml` — 配置
- `anyhow` — 命令层错误传播
- `thiserror` — 领域错误定义
- `directories` 或 `dirs` — 目录解析
- `sysinfo` — 进程存活校验
- `duct` — 启动外部命令
- `tracing` + `tracing-subscriber` — 日志

**不引入 `tokio`**：v0 全是同步操作（启动进程、读写文件、查 PID），无并发需求，async 会增加无谓复杂度。使用 `duct` / `std::process` 同步实现。

## 3. 配置与状态目录

- 配置文件：`~/.config/byteboy/config.toml`（XDG 风格）。
- 状态目录：`~/.local/state/byteboy/`，存放 `<id>.pid` 与 `<id>.log`。

**config 缺失时自动写入默认模板**（含示例 agents 与占位 model 段），再加载，并提示用户用 `boy config edit` 修改。

配置结构示例：

```toml
[agents]
claude = "claude"
codex  = "codex"
hermes = "hermes"
aider  = "aider"

[models.qwen36]
name    = "Qwen3.6"
command = "mlx_lm.server"
model   = "/Users/xxx/models/Qwen3.6"
host    = "127.0.0.1"
port    = 7101
```

对应 serde 结构：

```rust
struct Config { agents: BTreeMap<String, String>, models: BTreeMap<String, Model> }
struct Model { name: String, command: String, model: String, host: String, port: u16 }
```

## 4. CLI 命令表面

```
boy                              // 顶层帮助（clap 默认）
boy version                      // ByteBoy 0.1.0
boy agent list                   // 列出 agents，标 ✓/✗（PATH 检查）
boy run <agent>                  // 前台 exec 对应 CLI，透传后续参数
boy agent doctor                 // 逐个检查 agent CLI 是否在 PATH
boy model list                   // 列出已配置模型名
boy model start <id>             // 后台启动 mlx_lm.server
boy model stop <id>              // 终止对应进程
boy model restart <id>           // stop + start
boy model status                 // 遍历模型：Running host:port / Stopped
boy model logs <id>              // 实时跟随 <id>.log
boy doctor                       // 完整环境检查
boy config show                  // 打印 config.toml 内容
boy config edit                  // $EDITOR 打开 config.toml
```

## 5. Model 生命周期（PID + 日志文件）

- **start `<id>`**：查 `[models.<id>]`；若 `<id>.pid` 存在且进程存活则提示“已在运行”；否则用 `duct` 启动
  `mlx_lm.server --model <model> --host <host> --port <port>`，stdout/stderr 重定向到 `<id>.log`，后台运行（detached），写入 `<id>.pid`。
- **stop `<id>`**：读 `<id>.pid` → 终止进程 → 删除 pid 文件；进程已不存在则清理 pid 文件并提示。
- **status**：遍历所有配置模型，读 pid 并用 `sysinfo` 校验存活 → 输出 `<name>  Running  <host>:<port>` 或 `<name>  Stopped`。
- **logs `<id>`**：实时跟随 `<id>.log`（tail -f 等价）。
- **restart `<id>`** = stop + start。

## 6. Agent

- `[agents]` 为 name → command 映射。
- **list**：列出每个 agent，并用 PATH 查找标 ✓/✗。
- **run `<agent>`**：前台执行对应 command，继承 tty，透传用户后续参数；未配置该 agent 则报错。
- **doctor**：逐个检查 agent command 是否在 PATH。

## 7. Doctor

基础工具固定清单：`cargo`(Rust)、`python`、`git`、`uv`、`mlx`(检测 `mlx_lm`)、`comfyui`；再加上 `[agents]` 中所有 agent command。逐项做 PATH 检查，输出 `<tool>  ✓` 或 `<tool>  ✗ Not Installed`。

## 8. 错误处理与测试

- 命令层用 `anyhow::Result` 传播；领域错误用 `thiserror` 定义：`ConfigNotFound`、`ModelNotFound`、`AgentNotFound`、`ProcessNotRunning`、`PidFileError` 等。
- `main` 统一捕获并以非零退出码、友好信息输出。
- **可测试性**：纯逻辑抽成独立函数以便单测——
  - config TOML 解析（给定字符串 → Config）
  - pid 文件读写与解析
  - “命令是否在 PATH”判断（可注入 PATH）
  - doctor 检查项构造（工具清单生成）
  - model 启动命令行参数拼装（给定 Model → 参数向量）

## 9. v0 明确不做

Workflow、Skill、Prompt、MCP、Memory、Provider、RAG、LLM API、多模型路由、ComfyUI 自动调度、Web UI——全部推迟到 v1+。
