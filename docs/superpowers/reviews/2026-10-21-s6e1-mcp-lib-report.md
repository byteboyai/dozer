# S6e-1 `dozer-mcp` 共享库化实施报告

日期：2026-10-21。

计划：[2026-10-18-mcp-s6e1-lib.md](../plans/2026-10-18-mcp-s6e1-lib.md)。
前置：`feat/panel-agent-restore` 顶端 `adae877b`。

## 结论

分支 `feat/mcp-product`（worktree `../dozer-mcp-product`，从 `adae877b` 切出）完成 Dozer 侧 S6e-1。**未合并 `main`、未打 tag、未推送**；Task 6（Digger 升级钉版）按修订未执行。

提交：

```
a8fea45d feat(mcp): install 按产品取名、环境变量覆盖与同名保护
02f52562 feat(mcp): run_main、McpFeatures 与安静模式
2c25a0a8 feat(core): ProductId 的 mcp_bin/env_prefix 与已知会话变量列表
66ccc789 test(mcp): 共享库化前的默认产品黄金测试
```

## 基线（`adae877b`，实测）

- `cargo test --workspace --no-fail-fast` = **3436 通过 / 0 失败 / 5 ignored**。
- `dozer-app` 1251 / 2 ignored；`dozer-panel-agent` 71。
- `dozer-mcp` = **32**（lib 16 + 集成 16）；`cargo clippy --workspace --all-targets` = **41 条警告**（`dozer-mcp` 自身 0 条）。

## 各任务落地

- **Task 1 黄金测试**：`tests/fixtures/{tools_list_golden.json, initialize_golden.json, install_{claude,codex,opencode}_golden.*, serve_error_golden.txt}` + `tests/default_product_golden.rs`。夹具在 `66ccc789` 生成一次，此后**字节未变**（`git log -- tests/fixtures` 只有该提交）。
- **Task 2**：`ProductId::mcp_bin()` / `env_prefix()`；`paths::KNOWN_SESSION_ENVS`；`other_known_session_env_present(own, env)`。3 条新测试。
- **Task 3**：`features::McpFeatures`（5 组 12 工具、`all`/`none`/`tool_names`）；`DozerMcpServer` 改为可裁剪 `ToolRouter`（`#[tool_router(vis="pub")]` + 手写 `#[tool_handler] impl ServerHandler`），`get_info` 用注入的 `server_name`；`run_main(product, features)`、`run_for(product, features)`、`missing_session_message(product)`；安静模式按“本产品变量不存在且另一个已知产品变量存在且非空”触发。新增 `features_gating.rs`（6 条）与 `quiet_mode.rs`（5 条）。
- **Task 4**：`install` 三种格式的入口都增加条目名；`config_path_for_product`（环境变量名带产品前缀）；`run_for` / `run_at_with_exe_for`；`entry_is_ours` 同名保护（返回码 3，不写文件）；旧 `config_path_for` / `run` / `run_at_with_exe` 保留为默认产品薄封装。新增单元测试与 `coexistence.rs`（4 条：三种格式的两产品共存、Codex 注释/表保留、行内 `mcp_servers` 保留）。

## 测试与门禁（最终代码实测）

| 命令 | 结果 |
| --- | --- |
| `cargo build --workspace` | 退出码 0 |
| `cargo test --workspace --no-fail-fast` | 退出码 0；**3469 通过 / 0 失败 / 5 ignored**（+33） |
| `cargo test -p dozer-mcp` | 62 通过 / 0 失败（基线 32，+30） |
| `cargo clippy --workspace --all-targets` | 退出码 0；**41 条警告**（与基线相同） |
| `cargo fmt --check` | 退出码 1；仍只有 `main` 既有的 9 个文件，本期文件干净 |
| `bash scripts/check-i18n.sh` | 退出码 0 |
| `scripts/audit/check_panel_boundary.py` | 通过 |
| `cargo build -p dozer-mcp --release` | 退出码 0（打包链路可产出） |
| `cargo tree -p dozer-mcp --edges normal` | 无 `dozer-app` / `dozerd`（`dozerd` 仅 dev-dependencies） |

### 干净快照（`git archive HEAD`，独立 `target`）

- `cargo metadata --locked --offline`：退出码 0。
- `cargo test --workspace --no-fail-fast`：**3466 通过 / 3 失败 / 5 ignored**；3 条失败均为 `git_log` dogfood 测试（archive 无 `.git`），属环境原因；3466+3 = 3469 与 worktree 一致。
- `cargo test -p dozer-mcp`（同一快照、单独构建）：**62 通过 / 0 失败**。两种构建（`-p dozer-mcp` 与 `--workspace`，`serde_json` 的 `preserve_order` 统一结果不同）均全绿。
- `python3 scripts/i18n/check_migration.py`：退出码 0。
- `python3 scripts/audit/check_panel_boundary.py`：通过。

## 与计划的偏差（均有理由）

1. **默认服务器名保持 `rmcp`**：现状 `initialize` 的 `serverInfo.name` 是 `rmcp`，为了“默认产品字节不变”，`run_for` 对 dozer 仍传 `"rmcp"`；其它产品用 `product.mcp_bin()`（如 `digger-mcp`）。自定义名测试覆盖两者。
2. **JSON 黄金比较按键值语义（F1 返工后）**：`serde_json` 是否保留键序由构建里其它依赖的 feature 统一决定（`-p dozer-mcp` 与 `--workspace` 不同，发布构建启用 `preserve_order`）。产品代码**不做任何键排序**（`write_json` 恢复为直接 `to_string_pretty`），用户配置文件里无关键的相对顺序原样保留；`tools/list`、`initialize` 与三种 install 产物的黄金都只按 `Value` 递归规范键序后比较内容。夹具本身未重新生成。回归测试 `install_does_not_reorder_unrelated_keys` 在启用 `preserve_order` 的构建里断言非字典序输入原样保留（未启用时打印 skip 并注明原因）；`cargo test --workspace --lib install_does_not_reorder_unrelated_keys -- --nocapture` 实测执行了断言（无 skip），若重新引入排序会失败。
3. **安静模式仍构造 `Client` 值**：`DozerMcpServer` 字段需要它；`Client::new` 只保存 socket 路径，不连接。测试用不存在 socket 的 HOME 并断言没有创建/连接。
4. `run_at_with_exe_for(product, path, agent, install, exe)` 的 `path` 按计划是 `&Path`（没有 `Option`）；默认路径由 `run_for` 经 `config_path_for_product` 取。
5. 旧的 3 个三参数 install 处理器保留为 `#[cfg(test)]` 包装，既有 16 条单元测试原样通过；生产路径统一走带条目名的版本。

## F1 返工与审核观察收口

- **F1（阻塞）已修**：删除 `install::write_json` 里的全文件键排序，恢复 `serde_json::to_string_pretty` 直接写出；安装产物黄金改为与 `tools_list`/`initialize` 一致的 `canonical_json` 内容比较；新增 `install_does_not_reorder_unrelated_keys`（构建相关，见偏差 2）。`cargo test -p dozer-mcp` 62、`cargo test --workspace --no-fail-fast` 3469，两种构建均全绿。
- **观察 1（空的本产品会话变量）**：行为变化属实——旧 `run()` 把 `DOZER_SESSION_ID=` 当有效 id，新 `run_for` 按计划“存在且非空”视为缺失。已新增 `empty_own_session_variable_is_treated_as_missing`：空自己的变量且无别家 → 报错；空自己的变量 + 别家变量在 → 安静模式。
- **观察 2（计划点名的测试）**：`a_user_written_entry_with_the_same_name_is_not_overwritten` 已改回计划全名；补了 `uninstall_refuses_to_remove_a_foreign_same_named_entry`（三种格式）、`missing_empty_and_corrupt_files_behave_as_before`、`reinstalling_either_is_idempotent_and_does_not_touch_the_other`（三种格式，并核对 `other` 条目不变）。

## 手动抽检清单（写进报告，不阻塞）

整条 Dozer 链路完成后一次性验收：

- 开一个 Claude 会话，让 Agent 调 `list_memories` / `write_memory`，Project 面板里能看到结果；
- Agent 的 MCP 列表里 `dozer` 已连接且工具齐全；
- 已有的 `~/.claude.json` 等配置里的 `dozer` 条目内容不变。

## 未做

- 未合并 `main`、未打 `libs-*` tag、未推送；Digger 升级钉版（Task 6）待用户验收后另行安排。
