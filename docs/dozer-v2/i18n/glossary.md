# Dozer i18n 术语表

> 日期：2026-10-12。记录中英术语、品牌保留项与措辞修订。
> 规格 §6.1：英语资源为官方文案基准，首批全部官方资源要求英中 key 集及占位符集合一致。
> 迁移原则（计划 §1.1）：中文包以当前实际中文文案为初始值，保留语义、标点、换行及参数位置；
> 纠正明显文案问题须在本表记录旧值/新值/原因，不得借迁移批量改产品措辞。

## 1. 品牌与专有名词（保留原词，不翻译）

| 原词 | 处理 | 说明 |
|---|---|---|
| Dozer | 保留 | 产品名 |
| ByteBoy2077 | 保留 | 主题名 |
| Bytehost / bytehost | 保留 | 宿主平台库 |
| Byteui / byteui | 保留 | 组件库 |
| Bytegit / bytegit | 保留 | Git 库 |
| Claude / Codex / Opencode | 保留 | Agent 名，产品标识 |
| MCP | 保留 | 协议名 |
| Git / GitHub | 保留 | |
| SSH | 保留 | |
| SFTP | 保留 | |
| PTY | 保留 | |
| CodeMirror / vanilla-jsoneditor / Flyfish / PlantUML / Excalidraw | 保留 | 第三方控件/渲染器名 |
| SQL / SQLite / PostgreSQL / MySQL | 保留 | |
| JSON / JSONL / NDJSON / UTF-8 / UTF-16 | 保留 | 格式名 |
| Todo / Usage / Code Health / Group Chat | 英文包用英文；中文包用「待办」「用量」「代码健康度」「群聊」 | 见 §2 面板名 |

## 2. 面板与 Host 术语对照

| 中文（现有） | 英文（拟） | key 根（owner） |
|---|---|---|
| 文件 | Files | `dozer.files` |
| 待办 | Todo | `dozer.todo` |
| 项目 | Project | `dozer.project` |
| 数据库 | Database | `dozer.database` |
| SSH 主机 | SSH Hosts | `dozer.ssh` |
| 浏览器 | Browser | `dozer.web` |
| 代理 | Agent | `dozer.agent` |
| 群聊 | Group Chat | `dozer.group_chat` |
| 对话 | Conversations | `dozer.conversations` |
| 用量 | Usage | `dozer.usage` |
| 代码健康度 | Code Health | `dozer.code_health` |

Host owner = `dozer.host`；预览 chrome owner = `dozer.preview`。

## 3. 通用 UI 术语对照

| 中文 | 英文 | 备注 |
|---|---|---|
| 保存 | Save | |
| 取消 | Cancel | |
| 删除 | Delete | |
| 重试 | Retry | |
| 重载 | Reload | |
| 确认 | Confirm | |
| 关闭 | Close | |
| 复制 | Copy | |
| 粘贴 | Paste | |
| 移动 | Move | |
| 重命名 | Rename | |
| 新建 | New / Create | 按上下文 |
| 设置 | Settings | |
| 主题 | Theme | |
| 语言 | Language | |
| 跟随系统 | Follow system | 可翻译 |
| 英文 | English | 自称，不翻译 |
| 简体中文 | 简体中文 | 自称，不翻译 |

## 4. 措辞修订记录

| 位置 | 旧值 | 新值 | 原因 | 日期 |
|---|---|---|---|---|
| （暂无） | | | 迁移前不改产品措辞；发现明显问题在此登记 | |

## 5. 计数/复数约定

- 不引入 Fluent/ICU 复数语法（规格 §6.2）。
- 需数量变化的句子显式定义 `.zero` / `.one` / `.other` key，由调用点按数量选择；en/zh-CN 均具备相同 key 集。
- 中文对应词通常无复数变化，但 key 集需与英文对齐。
- 动态句子迁移整句模板，例如 `files.delete.completed = "已删除 {count} 个文件"`；不拆成「已删除」+ 数字 +「个文件」。
