# P1e agent 集成人工验收记录

**状态:待验收(2026-07-18 全量回归已绿:121 测试通过、clippy 零警告、fmt 干净;app 冒烟 8 秒无 panic)。**
**验收人:用户(甲方);验收权归用户,实施方(agent)不代签。**

## 清单逐项结果

| # | 验收项 | 结果 | 备注 |
|---|--------|------|------|
| 1 | 新 tab `cd` 几层 → tab 标题跟随目录名;`false` 回车 → 红字 `exit 1`,再跑一条命令提示消失 | 待验 | |
| 2 | `dozer-hook install` → settings.json 出现 7 事件条目且原有配置无损;重复执行无重复条目 | 待验 | |
| 3 | tab 里跑 `claude` 派活 → 胶囊绿"运行中"→ 提问/要权限紫"待输入"→ 回合结束金"回合毕" | 待验 | |
| 4 | 关 app 重开 → 胶囊状态恢复(registry 最新态经 SessionInfo 下发) | 待验 | |
| 5 | 停掉 dozerd 后在 Dozer 外的终端跑 claude → 无报错无卡顿(hook 静默) | 待验 | |
| 6 | `DOZER_SHELL_INTEGRATION=0` 起 dozerd → 新 tab 标题不再跟随 cwd;去掉后恢复 | 待验 | |
| 7 | `dozer-hook uninstall` → dozer 条目干净移除,他人配置无损 | 待验 | |

## 反馈 → 修复记录(按轮次)

(待验收反馈后填写)

## 结论

(全部通过后填写;规格 §3 需求 1 追加 P1e 达成标注)
