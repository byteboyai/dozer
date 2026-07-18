# Dozer P1f 设计：验收闭环纵向薄片

> 状态：设计稿,待用户审阅。切片方式（纵向薄片）与交付判定（回合毕且有变更）为用户已确认；其余裁决因用户暂离由实施方按"最小可逆"代拟,**逐条可推翻**。
> 上游:规格 §3 需求 3、§6 领域模型与数据流 2、§7 S2/S3;P1e 验收档留的挂点（TurnEnded、HookEvent.data 透传）。
> 起点:P1e 并入 main 后的工作区。

## 0. 薄片范围

**目标：把"定标 → 交付 → 验收 → 沉淀"最简闭环第一次跑通**，让 Dozer 的 dogfooding 从"终端+预览"升级为"带验收关口"。

**做**：goal.md 定标（文件承载）｜TurnEnded+有变更 → 交付横幅｜验收 tab（变更文件清单 + 标准勾选 + 意见 + 通过/打回）｜通过 → git ref 沉淀 + SQLite 验收记录｜打回 → 意见注回会话。
**不做（后续阶段）**：S0/S0b/S3 独立页面、diff 渲染视图（先给文件清单，点文件用 Flyfish 看现状）、机器预判自动跑可执行标准、演进史视图、多项目管理、transcript 交付声明解析。

## 1. 关键裁决（代拟，可推翻）

- **D1 项目=会话 cwd 所在 git 仓库**：`git rev-parse --show-toplevel` 认定；非 git 目录的会话不参与闭环（横幅永不亮）。一期单项目现实（dogfooding 即本仓）。
- **D2 定标走文件 `.dozer/goal.md`**，不做录入 UI：第一个非空行=目标；`- [ ]` 列表项=验收标准。**贴合规格 §6 存储裁决"文件态资产留项目仓库归 git"**，且录入姿势天然 dogfooding——在 Dozer 终端里让 claude 起草、用户改定。无此文件 → 横幅仍亮但验收 tab 提示"未定标,先写 .dozer/goal.md"（关口松紧属规格 §8 未决,薄片不设硬关口）。
- **D3 交付判定（用户已确认）**：`TurnEnded` 时对会话 cwd 仓库判"有变更"→ 终端栏亮金色横幅"交付待验收 [进入验收]"。纯问答回合不打扰。**"有变更"的精确定义**：`git status --porcelain` 非空（未提交改动）,或有沉淀 ref 时 `HEAD != refs/dozer/accepted/<max>`,或无 ref 时 `HEAD != 该会话上一次 TurnEnded 时记录的 HEAD`（GUI 内存记录,首个回合以会话 attach 时的 HEAD 为基线）。
- **D4 git 操作全走 git CLI、全在 GUI 侧**（规格"写路径驱动 git CLI";读路径薄片也用 CLI——`diff --numstat`+`status --porcelain`,gix 优化留后续）。dozerd 不碰 git。
- **D5 验收记录落 dozerd 侧 SQLite**（规格"结构性记录进 daemon 本地 SQLite";CLAUDE.md dozerd 职责表本就含"验收闭环存储"）。协议加 `Request::RecordAcceptance{repo, goal, criteria_checked, verdict, comment, ref_name, ts_ms}` → `Reply::Ok`。库表 `acceptances`,含 `acceptor` 字段（四期留门,一期恒 "user"）。查询接口留 P1g（演进史视图时再加）。
- **D6 沉淀=git ref**：通过时 `git update-ref refs/dozer/accepted/<n> HEAD`（n=现存 accepted ref 最大号+1）。工作区脏 → 红字阻止"有未提交变更,先让 agent 提交再沉淀"（沉淀物必须是提交,否则 ref 无锚点）。
- **D7 打回=意见注回来源会话 PTY**：向该会话 write `"[Dozer 验收打回] <意见>\n"`——claude 正在交互态即收到并继续；横幅熄灭,状态胶囊回 Running（下轮 TurnEnded 再判）。
- **D8 验收 tab 复用左二预览域 tab 机制**（规格 S2 全屏态推后）：内容=目标+标准清单（点击勾选,金勾）+变更文件列表（±行数,点击用 Flyfish 打开该文件）+意见输入（复用地址栏式自绘单行输入与键盘路由）+双动作:`通过·沉淀`（金）/`打回并注回`（红描边）。

## 2. 组件与数据流

```
TurnEnded(P1e 胶囊已有) ──GUI──▶ delivery.rs: git status/diff-numstat(会话 cwd 仓库)
    有变更 → 终端栏金横幅 [进入验收]
    点击 → 左二验收 tab: goal.rs 读 .dozer/goal.md + 文件清单 + 勾选态(内存)
    ├─ 通过 → git update-ref refs/dozer/accepted/<n> HEAD
    │        → Request::RecordAcceptance → dozerd SQLite(acceptances 表)
    │        → 横幅灭,tab 显示"已沉淀 v<n>"
    └─ 打回 → client.write(会话, "[Dozer 验收打回] <意见>\n") → 横幅灭
```

- `crates/dozer-app/src/goal.rs`（新）：`parse_goal(md: &str) -> Goal{title, criteria: Vec<String>}` 纯函数。
- `crates/dozer-app/src/delivery.rs`（新）：`repo_root(cwd)->Option<PathBuf>`、`changes_since_accepted(repo)->Vec<FileChange{path,added,removed}>`、`next_accepted_n(repo)->u32`、`is_dirty(repo)->bool`——均薄封装 git CLI（std::process::Command,GUI 侧 tokio spawn_blocking）。
- `crates/dozerd/src/acceptance.rs`（新）：rusqlite 存 `acceptances(id, repo, goal, criteria_checked, verdict, comment, ref_name, acceptor, ts_ms)`;库文件 `state_dir()/dozer.db`。
- `crates/dozer-core/src/protocol.rs`：`RecordAcceptance` 消息。
- workspace：横幅状态、验收 tab（PreviewPane 新 tab 类型或并列域,实施计划定）、勾选/意见/双动作消息。

## 3. 错误处理

- 非 git 仓库/git CLI 失败：横幅不亮,tracing 日志;验收 tab 内失败红字域内显示。
- goal.md 缺失/无标准行：验收 tab 提示定标缺失,通过按钮仍可用（薄片不设硬关口,规格 §8）。
- 工作区脏时点通过：红字阻止,不写 ref 不落库。
- SQLite 落库失败：红字提示但 git ref 已写成立（ref 是真相源,库是记录;P1g 补对账）。
- 打回时会话已死：红字"会话已结束,意见无处可注",意见保留在输入框。

## 4. 测试策略

Headless：goal.rs 解析（标题/标准/空文件/无标准）;delivery.rs 用 tempdir 建真 git 仓库测变更检测/numstat/next_n/dirty;acceptance.rs 落库与字段完整性;协议 roundtrip;横幅判定纯函数（TurnEnded×有无变更）。
人工验收清单（草案）：dogfooding 本仓——写 goal.md → 让 claude 改点东西 → 回合毕横幅亮 → 进验收 tab 勾标准 → 打回一次（意见出现在 claude 输入）→ 再通过 → `git for-each-ref refs/dozer/accepted` 见 v1 → 纯问答回合不亮横幅。

## 5. 备选方案（已否）

- **录入 UI 定标**：iced 侧多行表单成本高,且违背"资产归 git"存储裁决——文件承载更薄更正。
- **dozerd 算 diff**：daemon 碰 git 违背哑管道现状,且 GUI 侧已有会话 cwd 上下文。
- **每回合都算交付/显式声明才算**：用户已裁决取中间态（回合毕且有变更）。
