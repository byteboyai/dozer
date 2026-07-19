# P1f 验收闭环薄片人工验收记录

**状态:待验收(2026-07-19 全量回归已绿:136 测试通过、clippy 零警告、fmt 干净;app 冒烟 8 秒无 panic)。**
**验收人:用户(甲方);验收权归用户,实施方(agent)不代签。**

## 清单逐项结果（dogfooding 本仓）

| # | 验收项 | 结果 | 备注 |
|---|--------|------|------|
| 1 | 写 `.dozer/goal.md`（首行目标 + 两条 `- [ ]` 标准）→ Dozer 里跑 claude 改点东西 → 回合毕金横幅"交付待验收"亮 | 待验 | |
| 2 | 纯问答一回合（不改文件）→ 横幅不亮 | 待验 | |
| 3 | 点"进入验收" → 左二"验收" tab：目标/标准清单/变更文件（±行数）/意见框/双按钮；webview 不抢层 | 待验 | |
| 4 | 点标准行 → 金勾 ✓ 切换；点变更文件 → Flyfish 打开 | 待验 | |
| 5 | 意见框输入中文 → "打回并注回" → 意见出现在来源会话 claude 输入,验收 tab 关闭 | 待验 | |
| 6 | agent 提交全部变更 → 回合毕横幅亮 → 进验收 → 通过·沉淀 → "已沉淀 v1"；`git for-each-ref refs/dozer/accepted` 见 v1 指向 HEAD | 待验 | |
| 7 | 工作区留未提交变更时点"通过" → 红字阻止,ref 不写 | 待验 | |
| 8 | `sqlite3 ~/Library/Application\ Support/ai.byteboy.dozer/dozer.db 'select verdict,ref_name,acceptor from acceptances'` → accepted 记录,acceptor=user | 待验 | |

## 反馈 → 修复记录(按轮次)

(待验收反馈后填写)

## 结论

(全部通过后填写;规格 §3 需求 3 追加 P1f 达成标注)
