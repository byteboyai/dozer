# bytegit 审核修复

日期：2026-10-31。对应[审核报告](../../analysis/2026-10-31-bytegit-implementation-review.md)的 R1–R4。

## 当前落地状态

- R1/R2：已直接修改 Dozer `crates/dozer-git-runtime/`，增加全局唯一查询 ID、独立 running 注册表、执行任务持有的 semaphore permit，以及 watcher 启动结果/回调的实例隔离。
- R3/R4：[bytegit-fixes.patch](./bytegit-fixes.patch) 已在隔离源码副本实现并验证，基于 bytegit `4a4951b`。本会话只允许写 Dozer 和临时目录，bytegit 原仓库未写回，未提交、未发布新版本。

补丁只修改 bytegit `src/workspace_watch.rs`：

1. `need_rescan()` 优先于 Access 和路径过滤，强制完整重扫。
2. linked worktree 增加共享 `info/` 和 `objects/info/` 非递归监听，避免扫描整个对象树。
3. worker 持有 watcher，定期检查少量额外控制目录；删除时报告 degraded，目录创建/恢复后重新挂载并补扫；目录替换事件触发重新挂载。
4. 增加 rescan、监听目标覆盖及真实 linked worktree 配置/目录重建回归测试。

## 应用 bytegit 补丁

以下从 Dozer 根执行；先确认 bytegit 对应文件没有其它未提交改动：

```sh
git -C ../bytegit apply --check "$PWD/docs/patches/2026-10-31-bytegit-audit/bytegit-fixes.patch"
git -C ../bytegit apply "$PWD/docs/patches/2026-10-31-bytegit-audit/bytegit-fixes.patch"
```

随后在 bytegit 根运行：

```sh
cargo fmt --check
cargo test --locked
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
```

`bytegit-regressions.patch` 和 `dozer-regressions.patch` 是审核时针对原实现的失败反例，仅保留历史证据。不要先应用这些历史补丁再应用修复补丁；修复已包含正式回归。

## 验证范围

隔离工作区：`/private/tmp/bytegit-audit.eGgEFr`，使用当前源码及本地缓存依赖，离线执行：

- bytegit 默认测试：157 passed。
- bytegit all-features：223 passed。
- runtime：14 单测、15 集成测试、1 doctest 通过。
- 两个库的 all-targets/all-features clippy `-D warnings` 和修改文件 rustfmt 检查通过。
- bytegit 补丁对原仓库执行 `git apply --check`，只检查可应用性、不写入原仓库。

真实 linked worktree 的 exclude/alternates 更新及共享 info 目录删除、重建后继续接收配置事件已在本机 macOS 验证。Linux/Windows 尚未执行；完整 Dozer/Digger 产品集成及 T6–T13 不在这次四项缺陷修复范围内。
