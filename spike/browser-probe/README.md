# browser-probe

Digger 浏览器面板 Task 0 的一次性探针(`docs/superpowers/plans/2026-10-28-digger-browser-panel.md`),随时可删。
自成工作区(已在仓库根 `Cargo.toml` 的 `exclude` 里),不进 CI。结果与结论见
`digger/docs/superpowers/reviews/2026-10-28-browser-panel-spikes.md`。

```bash
cd spike/browser-probe
cargo run                                 # 自动跑 S4 / S1 / S3 / S6(约 1 分钟,会闪一个小窗口),打印 PROBE 行
cargo run -- --s3-live                    # S3 活体:页面真的调 getUserMedia(见报告;需要本机 127.0.0.1:18765 起一个 http 服务)
cargo run -- --interactive                # S2 手动:在 webview 里试键盘 / 输入法
cargo run -- --interactive --edit-menu    # S2 手动:同上,先装一个 Edit 菜单(Cut/Copy/Paste/Select All)
```

只访问本地 `file://` 与回环地址,不出网;不会在没有代理拒绝的情况下调用摄像头(自动模式不调用 getUserMedia)。
夹具文件写在 `/tmp/browser-probe/`。
