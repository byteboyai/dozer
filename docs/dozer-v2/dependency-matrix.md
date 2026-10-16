# 共享依赖版本矩阵(dozer / digger)

> 维护对象:Dozer 与 Digger 共同消费的 bytehost / byteui / bytegit / dozer 库 crate 的钉法。
> 任一共享库发版后,本表是"两边该升到哪"的唯一对照。生成时间:2026-10-16。

## 1. 矩阵

| 仓库 | crate | 钉法 | 说明 |
|---|---|---|---|
| `byteboyai/bytehost` | `bytehost-core`/`-chrome`/`-desktop`/`-webview`/`-apps`/`-client`/`-panel` | tag `v0.4.1` (`79212c08`) | 应用宿主平台库;dozer 与 digger 同日一起升 |
| `byteboyai/byteui` | `byteui` | tag `v0.4.4` (`4c539c7b`) | iced 0.14 组件库;iced 版本必须与之同升 |
| `byteboyai/bytegit` | `bytegit` | tag `v0.7.0` (`0a3d2ab3`) | git 底层能力(dozer 侧消费;digger 经 dozer 传递) |
| `byteboyai/dozer` | `dozer-model`/`dozer-services`/`dozer-panel-usage`/`dozer-store-project` | tag `libs-2026-10-16b` (`2cddd564`) | 库 crate 快照 tag;`v0.5.x` 是 App 发布 tag,不混用 |

> dozer 当前工作副本的 `bytehost` 已到 `v0.4.1`、`byteui` 到 `v0.4.4`,库快照 tag 为 `libs-2026-10-16b`。

## 2. 规则

1. **同日一起升:** 升级 bytehost / byteui 时,dozer 与 digger 必须在同一天一起升级。否则 digger 会解析出两份 `PanelId`(类型互不兼容)或让 byteui 的全局主题状态分裂(用量面板的主题会错)。
2. **门禁:** dozer 侧 `scripts/check-shared-deps.sh`(共享库依赖守卫:本地 patch / lock 指向本地 / 远端 tag 缺失 / 多份版本 / 落后提示;pre-push 钩子需 `git config core.hooksPath .githooks` 启用)与 digger 侧 `scripts/check-host-deps.sh`(共享宿主去重门禁)是硬门禁,CI/提交前都要跑。
3. **本地 patch 只用于联调,不提交:** dozer 的 `.cargo/config.toml` 里 `[patch.".../byteui"]` / `[patch.".../bytegit"]` 只用于本地联调,不提交;digger 已删除全部 `[patch]` 段,不再保留任何本地路径 patch。

## 3. 单一副本校验

依赖图中每个共享 crate 只能有一份。校验命令:

```bash
cargo tree -p <root-crate> -i byteui          # 期望只有一行
cargo tree -p <root-crate> -i bytehost-core   # 期望只有一行
cargo tree -p <root-crate> -i dozer-model     # 期望只有一行
```

dozer 侧等价门禁:`bash scripts/check-shared-deps.sh --rev HEAD`;digger 侧:`bash scripts/check-host-deps.sh`。
