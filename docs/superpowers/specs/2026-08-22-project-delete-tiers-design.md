# 删除项目三级方案 Design

**Status:** 已批准设计，待写实现计划。

## 背景

项目面板"删除项目"按钮目前是 `Message::DeleteProject => {}` 空占位（`project-scaffold-repair` 落地时特意标注"独立设计,不在本次范围内"，见 `crates/dozer-app/src/extensions/project.rs` 里那行注释）。用户诉求：点击后弹出三个递增破坏程度的范围选项：

1. 只删 dozer 与项目的关联 + dozer 的缓存文件（`.dozer/`）。
2. 在 1 的基础上，再删所有 agent（Claude/CodeBuddy/OpenCode）为这个项目缓存的历史数据。
3. 在 2 的基础上，再删项目文件本身与版本仓库（`.git`）。

调研确认：这个仓库目前**没有任何删除项目的机制**——`dozer-core::protocol` 没有对应的 `Request`，`dozerd` 的 `ProjectStore`/`TranscriptStore` 都只有插入/查询方法，没有删除方法。三个层级都是新写的后端能力。同时确认了一个关键的既有安全惯例：`files.rs` 的文件树删除功能用的是 `trash::delete`（移入系统回收站，可找回），不是 `std::fs::remove_dir_all`（真正永久删除）——这次三个层级统一沿用这个模式（讨论中已拍板）。

## 范围边界

- 只做删除相关的新增：`Request::RemoveProject`/`Request::DeleteProjectTranscripts`、`ProjectStore::remove`、`TranscriptStore` 新增的按项目删除方法、`.dozer`/agent 缓存目录/项目根目录的 `trash::delete`、三选一确认弹窗。
- 不改动"新建项目"/"修复项目"已经落地的 ensure 逻辑（`project_scaffold.rs`），两者除了共用"项目根目录路径"这个输入外没有代码耦合。
- 不引入真正永久删除（`remove_dir_all`）——三个层级统一走 `trash::delete`（讨论中已拍板）。
- 不要求输入项目名做二次确认——用跟 `ssh.rs`/`files.rs` 现有删除功能一样的取消/确认二次确认弹窗（讨论中已拍板）。

## 架构与数据流

### 1. 三个层级，严格递增

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeleteScope {
    /// 只删 dozer 登记 + `.dozer/` 缓存。
    DozerOnly,
    /// 含 `DozerOnly`，再删三家 agent 为这个项目缓存的历史数据。
    WithAgentCache,
    /// 含 `WithAgentCache`，再删项目文件本身（含 `.git`）。
    WithProjectFiles,
}
```

### 2. dozerd 侧新增（先执行，失败即整体中止，不碰文件系统）

- `Request::RemoveProject { id: i64 }` → `Reply::Ok`。所有层级都要发。新增 `ProjectStore::remove(&self, id: i64) -> Result<()>`，跟现有 `rename`（`projects.rs:161`）同一套"`conn.execute` 影响行数为 0 就 `bail!`"判据：

  ```rust
  pub fn remove(&self, id: i64) -> Result<()> {
      let conn = self.conn.lock().expect("db lock");
      let affected = conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
      if affected == 0 {
          anyhow::bail!("项目 id={id} 不存在");
      }
      Ok(())
  }
  ```

- `Request::DeleteProjectTranscripts { cwd: String }` → `Reply::DeletedTranscripts { conversations: u32 }`。只有层级 `WithAgentCache`/`WithProjectFiles` 才发。`conversations`/`conversation_turns` 两张表按 `dir` 列过滤（跟 `list_conversations_in`，`mod.rs:327`，的过滤口径一致，`dir` 就是项目 `cwd`），**没有外键级联**，删除顺序必须是先子表再主表：

  ```rust
  pub fn delete_project_transcripts(&self, dir: &str) -> Result<u32> {
      let conn = self.conn.lock().expect("db lock");
      conn.execute(
          "DELETE FROM conversation_turns WHERE conversation_id IN \
           (SELECT conversation_id FROM conversations WHERE dir = ?1)",
          [dir],
      )?;
      let affected = conn.execute("DELETE FROM conversations WHERE dir = ?1", [dir])?;
      Ok(affected as u32)
  }
  ```

两个新 `Request`/`Reply` 变体、`server.rs` 里的处理分支、`dozer-client` 里的包装方法，三层都照抄 `RenameProject`（`protocol.rs:318-321`）→ `server.rs:268-278` → `dozer-client/src/lib.rs:166-178`）这条既有的"协议变体 → server match 分支调 store 方法 → client 异步包装"链路,不引入新模式。

### 3. 文件系统删除（dozerd 两步都成功后才开始，客户端侧执行，各步独立、尽力而为）

- 层级 `DozerOnly` 起：`trash::delete(repo.join(".dozer"))`。
- 层级 `WithAgentCache` 起：对 `agent_paths::{claude,codebuddy,opencode}_project_dir_in(home, repo)` 三个目录，存在的都 `trash::delete`。
- 层级 `WithProjectFiles`：`trash::delete(repo)`（项目根目录本身，含 `.git`）。

全部包进一次 `tokio::task::spawn_blocking`（复用 `files.rs::Message::DeleteConfirm` 已经用过的"`trash::delete` 塞进 `spawn_blocking`"手法，`files.rs:640-669`），每一步独立捕获错误存进 `Vec<String>`，一步失败不阻塞其它步骤继续跑，全部跑完后一次性带着错误列表发回一条完成消息。

### 4. 触发流程

面板状态新增 `delete_pending: Option<DeleteScope>`（`Some` = 弹窗开着，值是当前选中的层级；跟项目脚手架的 `scaffold_report` 字段并列，不复用同一个字段）。

1. "删除项目"按钮 → `Message::DeleteProjectRequest`，`delete_pending = Some(DeleteScope::DozerOnly)`（默认最轻层级，用户主动选才会加重）。
2. 弹窗里三个单选行 → `Message::DeleteProjectScopeSelect(DeleteScope)`，改写 `delete_pending` 里的值（弹窗还开着，不关闭）。
3. 取消 → `Message::DeleteProjectCancel`，`delete_pending = None`。
4. 确认 → `Message::DeleteProjectConfirm`：`let Some(scope) = ws_state.delete_pending.take() else { return };`。若这个项目当前是打开的 tab，先调用 `App::project_tab_close` 同款清理（`stash_active_panel_layout`/`take_project_tab`/`close_all_tabs_for_switch`，`app.rs:4911-4941`）关掉它，避免删除过程中还有 UI 引用这个项目；再依次跑 dozerd 两步（按 `scope` 决定要不要发 `DeleteProjectTranscripts`）——任一失败就中止，emit 一条带错误信息的完成消息，不碰文件系统；两步都成功才跑文件系统三步（按 `scope` 决定跑到哪一步），最后 emit 完成消息（错误列表为空 = 全部成功，弹窗直接关闭；非空 = 展示错误，弹窗关闭但错误信息保留展示一段时间，复用项目脚手架"面板内联状态文字"的展示位置和视觉，不新开 UI 组件）。

弹窗渲染沿用 `ssh.rs::delete_confirm_popup`（`ssh.rs:1208`）/`files.rs::delete_confirm_popup`（`files.rs:1692`）的卡片+取消/确认按钮模板，新增三个单选行——单选点的视觉直接复用 `ssh.rs` 已有的 `radio_dot` 组件（选中态 GOLD 实心描边，未选中态空心 BORDER 描边），不用重新画一套。

## 错误处理

- dozerd 两步（`RemoveProject`、层级 ≥ `WithAgentCache` 时的 `DeleteProjectTranscripts`）任一失败 → 整个流程中止，不碰任何文件系统，错误消息原样展示，弹窗状态可以重试（不是关闭后不可逆的失败）。
- 文件系统三步互相独立、尽力而为——一步失败不阻塞其它步骤继续跑，最后把所有失败原因拼成一段文字展示。
- "项目已经不存在于 `projects` 表"（比如被并发另一处删过）落进 `ProjectStore::remove` 的"影响行数=0"分支，按上面"dozerd 侧失败即中止"统一处理——不会出现"数据库记录删了、文件系统还没删"反过来的顺序倒置，因为文件系统步骤严格排在 dozerd 两步都成功之后才开始。

## 测试

- `ProjectStore::remove`：dozerd tempdir 测试，`open`→`remove`→`list` 确认目标项目不在列表里；对不存在的 `id` 调用 `remove` 确认返回错误。
- `TranscriptStore::delete_project_transcripts`：dozerd tempdir 测试，`ingest_session` 两个不同 `dir`（模拟两个项目）的 fixture，删一个 `dir`，确认只有目标项目的 `conversations`/`conversation_turns` 行被清空、另一个项目的数据完好（`get_conversation_turns` 还能查到）。
- 协议层新增的 `RemoveProject`/`DeleteProjectTranscripts` 两对 `Request`/`Reply`：roundtrip 测试，写法同现有 `conversation_protocol_types_roundtrip`/`backfill_project_transcripts_protocol_types_roundtrip`。
- `DeleteScope` 状态机（`DeleteProjectRequest`/`DeleteProjectScopeSelect`/`DeleteProjectCancel`/`DeleteProjectConfirm` 对 `delete_pending` 的状态流转）：纯状态单测，写法同项目脚手架的 `scaffold_done_stores_report_only_when_visible`。
- 真正的 `trash::delete` 调用路径：**不做单测**——`files.rs` 现有的删除功能同样只测到 `DeleteRequest`→`DeleteCancel`（`files.rs:2083`），从没测过真正调 `trash::delete` 那条路径（调研确认过），这次三级删除延续同一个现状，最终靠人工 GUI 走查确认。

## 已知取舍（不在本设计范围内，记录避免以后重新讨论）

- 统一用 `trash::delete`，不提供"真正永久删除、跳过系统回收站"的选项——如果以后有人明确需要这个，是一个新的、独立评估的功能（要考虑回收站容量、批量删除场景等），不是这次三选一里悄悄加一档。
- 不要求输入项目名做二次确认——`trash::delete` 本身可找回，弹窗确认的摩擦力止步于现有 `ssh.rs`/`files.rs` 两处删除功能的标准（取消/确认两个按钮），不因为"项目"这个删除对象比"一个文件"更重而单独加码。
- `conversations`/`conversation_turns` 两张表没有外键级联，删除顺序（先删 `conversation_turns` 再删 `conversations`）靠调用方代码顺序保证，不是数据库层面强制的——如果以后这两张表的 schema 改动引入了级联删除，`delete_project_transcripts` 里手动排的两条 `DELETE` 语句需要跟着重新评估要不要简化。
