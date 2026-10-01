# Dozer V2：Iced → GPUI 迁移评估

> 完整评估与全部实测在独立研究项目：`/Users/chrischiang/Projects/CoralProjects/byteboy/gpui-lab/docs/dozer-v2-GPUI迁移评估.md`
>
> **决策（2026-10-01，用户）：Dozer v2 不更换为 GPUI，保留 iced + CodeMirror。**
> 理由：额外约 55k 行代码（估算）与体积增加（约 400MB 为调试版数字，发布版未测），代价有点大。
>
> 评估的副产品里对 Dozer 仍有用的：CodeMirror + 官方 LSP 客户端可以提供补全与跳转（服务端放在 `dozerd`）；
> 双击同词高亮补一行 `highlightSelectionMatches()`；非严格 UTF-8 一律只读、保存恒拒绝的规则必须保留。
