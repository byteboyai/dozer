# ByteUI 审核修复补丁

日期：2026-10-31。Dozer 侧改动已直接写入当前工作区。当前会话只允许写 Dozer，
ByteUI / Digger 的改动在 `/private/tmp/byteui-review-fixes` 副本完成，补丁随后已由用户应用到 sibling 仓库。已确认两个补丁均可反向检查，所有改动文件与验证副本逐字节一致。

补丁基于审核时两个仓库的**当前工作区内容**（包含既有未提交开发结果），不是基于 HEAD。
不要 reset 既有开发结果再应用补丁。两个补丁须与当前 Dozer 改动配套使用：Dozer 的
`with_animation_paused` 调用依赖 ByteUI 补丁新增接口。

## 内容

- `byteui.patch`：提前重绘保留原期限；窗口视图构建的嵌套暂停作用域；主题测试使用
  RAII 恢复颜色快照/revision、字体、几何及缩放原始状态；候选 CI 在两侧仓库内更新临时锁后
  生成依赖图和 metadata，失败阻断；Dozer/Digger ref 改为必填。
- `digger.patch`：主窗口焦点消息带窗口身份，失焦/最小化/退出时暂停所有后代加载动画，
  增加窗口身份隔离测试。失焦不会冒充最小化，避免影响现有 WebView 生命周期策略。
- Dozer 工作区：主窗与浮窗视图构建传递动画暂停状态，继承到共享面板；浮窗使用自己的状态。

## 应用

在各仓库根目录执行（把 `<补丁目录>` 替换为本目录绝对路径）：

```bash
git apply --check <补丁目录>/byteui.patch
git apply <补丁目录>/byteui.patch
```

Digger 仓库同理使用 `digger.patch`。本次已对两个原始 sibling 工作区执行 `git apply --check`，均通过。
应用已完成，后续需继续按计划升级 ByteUI tag/两侧锁文件；补丁不代替发布升级。

## 验证与限制

- ByteUI 副本：133 项 lib 测试通过；fmt、clippy all-targets（deny warnings）通过。
- Dozer / Digger 副本：以候选 ByteUI path patch 完成各自 `cargo check -p … --tests`。
  锁文件仅在副本更新，未改原始工作区锁文件；不是固定发布 tag 的 clean 构建证明。
- Digger 副本：`product::tests::review_lifecycle` 22 项测试通过，包含新增焦点归属测试与现有失焦/最小化生命周期回归。
- CI：YAML 解析、全部 run 块 `bash -n` 通过；使用模拟 cargo 实际执行两侧 shell 步骤，
  确认读取当前仓库配置、临时锁先更新、解析图/metadata 生成成功；tree 返回 17 时任务失败，
  成功和失败均恢复原锁文件。
- 尚未执行远端 Actions、真实窗口的菜单销毁/切换验证及动画性能测量；不宣称这些验收通过。
- 本轮未改变菜单路由：上次审核提出的异步消息窗口归属是待复现风险，不是已确认缺陷。
