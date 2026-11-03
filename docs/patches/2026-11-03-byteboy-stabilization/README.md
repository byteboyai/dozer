# 稳定化补丁包

这些补丁已在隔离候选副本实现、验证，并通过原仓库 `git apply --check`。ByteUI、ByteHost、Digger 原仓库不可写，**尚未应用或发布**。Dozer 的门禁与规范修改已直接位于当前工作区，`dozer-release.patch` 仅包含候选依赖升级。

基线与 SHA256 见 [manifest.json](manifest.json)。不要把补丁成功应用等同于完成发行验收；包内没有伪造的发行 Cargo.lock。

## 补丁

- `byteui.patch`：预备 v0.5.2；发布已经存在于本地 eb08e57 的 NotebookTabs；候选 workflow 补齐 file-io/git-runtime 映射。
- `bytehost.patch`：预备 v0.4.22 并统一 ByteUI；测试进程 RAII/有限等待；两消费方 PR/发布联合检查；缺消费方失败、结束时恢复 lock；3 项脚本自检。
- `digger.patch`：完整锁定依赖图门禁及 15 项测试；精确 compiler-artifact 的实机自检入口、父进程超时回收及 4 项测试；CI 接线；候选共享版本清单。路径适配器仅允许纯 bytehost-apps 身份契约，仍禁止 GUI。
- `dozer-release.patch`：两项共享依赖版本升级；需等对应标签实际发布后再生成正式锁文件。

## 落地顺序

1. 核对原仓库基线/未提交变更，再 `git apply --check` 和应用对应补丁。ByteUI 本地 eb08e57 与 Dozer 本地 6e389a30 含尚未发布的先前提交，发布前必须包含它们；补丁不是对远端旧主分支的完整替代。
2. 验证 ByteUI，提交并发布新的 v0.5.2。标签名发布前再核对，已有标签不得移动。
3. 验证 ByteHost，提交并发布 v0.4.22。正常 macOS 进程身份查询环境复验孤儿恢复；本沙箱的 3 项失败没有被跳过。
4. Dozer 纳入当前门禁/规范变更，应用 release patch；在无个人 `.cargo` patch 的干净副本解析正式远端依赖并生成 lock，运行 locked 检查和回归；发布统一 libs-2026-11-03a 快照。
5. Digger 解析已发布依赖，替换原来 24 个共享包缺 git source 的 lock；在干净副本运行 locked 全目标、共享依赖与宿主边界门禁及核心测试。
6. 两产品实际关键路径与发行性能通过后，才退出稳定化批次。

生成正式 lock 时使用无个人 patch 的源码副本，将结果带回对应工作树；不删除开发者个人配置，不将候选 path 锁复制成发行锁。本包的未来标签清单在标签发布前无法单独完成无 patch 构建，这是明确的依赖顺序，不能用本地候选通过掩盖。

## 验证类型

候选验证临时沿用已存在的 git tag 作为解析起点，将全部相关共享 crate 指向隔离源码，并显式更新 ByteUI 到 0.5.2；此后两产品运行 `cargo check --locked --offline -j 1 --workspace --all-targets`。这证明所选源码 API 组合可编译，**不证明未来标签已发布或正式 lock 已生成**。

实机自检的 4 项 Python 测试只验证入口/超时/产物选择，不能代替真实窗口、IME、焦点或绘制验收。
