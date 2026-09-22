# Goose adapter spike

记录 Goose 接入 Dozer 的 contract 核实结果。**状态**:部分完成——CLI flags、
插件发现目录、`plugin.json`/`hooks.json` 格式、事件集与 payload schema 已对照
goose 1.51.0 的 `--help` 输出 + 官方 hooks 文档核实;但**真实 hook 事件的原始
stdin JSON 尚未在本机捕获**(`goose doctor` 报告未配置 LLM provider,跑不起
真实会话)。生产 parser 依据官方文档的 payload schema 落地,field 名与示例均
取自 `https://goose-docs.ai/docs/guides/context-engineering/hooks/`,不是猜测;
配好 provider 后应按下面 capture 步骤补一份真实 fixture 复核。

## 环境

```text
goose --version → 1.51.0
```

## CLI contract（已核实）

- 交互入口:`goose session`（`session` 是交互会话命令,`s` 是别名）。
- 一次性任务:`goose run`。关键 flags(用于 headless 总结/派发,spec D7):
  - `--text <TEXT>`:以参数形式传入指令(不用 stdin)。
  - `--no-session`:不创建/复用会话文件(总结场景用)。
  - `-q, --quiet`:只输出模型回复到 stdout。
  - `--with-extension <COMMAND>`:挂 stdio 扩展(可挂 `dozer-mcp serve`,首期
    不启用,见设计 D8)。
- `--no-session`/`--quiet`/`--text` 三者都存在于 1.51.0,spec D7 的
  `goose run --no-session --quiet --text <prompt>` 命令形状成立。

## 插件发现与 hooks 格式（已核实）

- 用户级插件目录:`~/.agents/plugins/<plugin-name>/`(project 级是
  `<project>/.agents/plugins/<plugin-name>/`)。
- 每个插件:`plugin.json`(name/version/description)+ `hooks/hooks.json`。
- `hooks.json` 顶层 `hooks` 对象,event 名 → 规则数组 → 每条规则 `matcher`
  (可选正则)+ `hooks` 数组 → 每个 action `{type:"command", command, timeout}`。
- hook command 由 `sh -c` 执行,stdin 传入事件 JSON,`${PLUGIN_ROOT}` 可用。
- 单行 payload 字段:`event`、`session_id`、`matcher_context`、`tool_name`、
  `tool_input`、`tool_call_id`、`message`、`last_assistant_message`、
  `working_dir`(仅 tool 事件)、`decision`/`policy_evaluated`/`cause`
  (仅 `PreToolUseResult`)。
- 事件集(比 spec D2 的八类更多):`SessionStart`/`SessionEnd`/`Stop`/
  `UserPromptSubmit`/`PreToolUse`/`PreToolUseResult`/`PostToolUse`/
  `PostToolUseFailure`/`BeforeReadFile`/`AfterFileEdit`/`BeforeShellExecution`/
  `AfterShellExecution`。Dozer 只注册前八类里的 D2 名单(不含
  `PreToolUseResult` 和 `Before*`/`AfterShellExecution`)。
- **cwd 来源**:payload 没有稳定的全局 cwd 字段(`working_dir` 只在 tool 事件
  出现),所以 journal 的 cwd 取 hook 子进程自己的 `current_dir()`——Goose 用
  `sh -c` 起 hook、继承会话工作目录(见设计 D3/D4)。

## 工具 input keys(内置 developer 工具)

`developer__shell`(`command`/`timeout_secs`)、`developer__write`(`path`/
`content`)、`developer__edit`(`path`/`before`/`after`)、`developer__tree`
(`path`/`depth`)、`developer__read_image`(`source`/`crop`)。`AfterFileEdit` 的
`matcher_context` 直接是文件路径,`PreToolUse`/`PostToolUse` 的
`matcher_context` 是 tool name。

## 待补:真实 payload 捕获

配好 provider 后:

```bash
mkdir -p ~/.agents/plugins/dozer-spike
cp plugin.json hooks/hooks.json ~/.agents/plugins/dozer-spike/
chmod +x capture.sh
goose session   # 在 DOZER_SESSION_ID=spike-session 下跑真实会话
```

`capture.sh` 会把每事件的原始 stdin JSON 追加到 `~/.dozer-spike-capture.log`。
覆盖:纯文本回复、成功工具、失败工具、文件编辑、中断、正常退出。把去敏后的
fixture 存进 `crates/dozer-hook/fixtures/goose-*.json`,并回填本 README 的
"已核实"与 parser 的字段名是否一致。
