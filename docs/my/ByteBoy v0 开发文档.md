# ByteBoy v0 开发文档

> Version: v0.1  
> Language: Rust  
> Platform: macOS (Apple Silicon First)

---

# 1. 项目定位

ByteBoy 是一个 AI 开发环境管理 CLI。

v0 的目标不是实现 Agent，也不是 Workflow，而是统一管理本地 AI 开发环境。

主要负责：

- Agent 管理
- MLX 模型管理
- 环境检查（Doctor）
- 配置管理

未来将逐步扩展到：

- Skill
- Workflow
- MCP
- Provider
- AI 自动化工作流

---

# 2. 技术栈

- Rust 2024 Edition
- clap（CLI）
- tokio（异步）
- serde
- toml
- anyhow
- tracing
- directories
- sysinfo（查询进程）
- duct（执行外部命令）

---

# 3. Workspace 结构

text byteboy/  Cargo.toml  crates/      byteboy-cli/     byteboy-core/     byteboy-agent/     byteboy-model/     byteboy-config/     byteboy-doctor/

各模块职责：

| 模块 | 职责 |
|------|------|
| byteboy-cli | CLI 命令解析 |
| byteboy-core | Context、Error、Logger 等公共模块 |
| byteboy-agent | Agent 管理 |
| byteboy-model | MLX 模型管理 |
| byteboy-config | 配置读取 |
| byteboy-doctor | 环境检查 |

---

# 4. 配置文件

默认位置：

text ~/.config/byteboy/config.toml

示例：

toml [agents]  claude = "claude" codex = "codex" hermes = "hermes" aider = "aider"  [models.qwen36]  name = "Qwen3.6" command = "mlx_lm.server" model = "/Users/xxx/models/Qwen3.6" host = "127.0.0.1" port = 7101  [models.coder]  name = "Qwen-Coder" command = "mlx_lm.server" model = "/Users/xxx/models/Qwen-Coder" host = "127.0.0.1" port = 7102

---

# 5. CLI 总览

bash boy

输出：

text ByteBoy AI CLI  Commands  run agent model doctor config version

---

# 6. Agent 管理

## 列出 Agent

bash boy agent list

输出：

text Available Agents  ✓ claude ✓ codex ✓ hermes ✓ aider

---

## 启动 Agent

bash boy run claude

bash boy run codex

bash boy run hermes

bash boy run aider

本质就是执行对应的 CLI。

---

## Agent 环境检查

bash boy agent doctor

输出：

text Claude CLI      ✓ Codex CLI       ✓ Hermes          ✓ Aider           ✓

---

# 7. Model 管理

## 查看模型

bash boy model list

输出：

text Qwen3.6 Qwen-Coder

---

## 启动模型

bash boy model start qwen36

执行：

bash mlx_lm.server \     --model xxx \     --host 127.0.0.1 \     --port 7101

后台运行。

---

## 停止模型

bash boy model stop qwen36

结束对应 mlx_lm.server 进程。

---

## 重启模型

bash boy model restart qwen36

---

## 查看模型状态

bash boy model status

输出：

text Qwen3.6  Running  127.0.0.1:7101

---

## 查看日志

bash boy model logs qwen36

输出实时日志。

---

# 8. Doctor

检查整个 AI 开发环境：

bash boy doctor

示例：

text Rust            ✓ Python          ✓ Git             ✓ uv              ✓  Claude CLI      ✓ Codex CLI       ✓ Hermes          ✓  MLX             ✓ ComfyUI         ✓

未安装：

text Claude CLI      ✗ Not Installed

---

# 9. Config

查看配置：

bash boy config show

编辑配置：

bash boy config edit

默认打开：

text ~/.config/byteboy/config.toml

---

# 10. Version

bash boy version

输出：

text ByteBoy 0.1.0

---

# 11. v0 不实现内容

以下内容全部延期到 v1：

- Workflow
- Skill
- Prompt
- MCP
- Memory
- Search
- Writing
- Product Workflow
- Provider
- RAG
- ChatGPT 接入
- Claude API
- ComfyUI 自动调度
- 多模型自动路由

v0 只关注 AI 环境管理。

---

# 12. v1 规划

新增：

bash boy skill search boy skill write boy workflow writing

Workflow 示例：

text Writing Workflow  Search     ↓ Outline     ↓ Draft     ↓ Review     ↓ Illustration

Skill 统一接口：

rust trait Skill {     async fn execute(ctx: Context) -> Result<()>; }

---

# 13. 开发原则

1. CLI First
2. Configuration over Code
3. Small Modules
4. One Command, One Responsibility
5. 所有命令可单独测试
6. 不依赖 Web UI
7. 优先支持 macOS（Apple Silicon）
8. 后续兼容 Linux
9. Trait 优先设计
10. 保持简单，避免过度设计

---

# 14. 后续路线图

## v0：AI Environment Manager（当前）

- Agent 管理
- MLX 模型管理
- 配置管理
- Doctor

---

## v1：Skill

增加：

- Search Skill
- Writing Skill
- Coding Skill
- Review Skill

---

## v2：Workflow

增加：

- Writing Workflow
- Product Workflow
- Research Workflow

---

## v3：Provider

统一接入：

- ChatGPT
- Claude
- Qwen
- MLX
- ComfyUI

---

## v4：Automation

实现真正的 AI 自动化开发与写作平台。