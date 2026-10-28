# dozer-browser-exposure

Dozer 浏览器面板暴露面的复现页(一次性,随时可删)。页面自动运行,不需要点击:报告 `window.ipc` 是否可见、注入脚本留下的全局、
对 `dozer://` 的子资源请求是否到达协议处理器,20 秒后把自己顶层导航到 `dozer://review-trace/data.json`,再 3 秒后写一行
"仍在本页"——能写出这一行说明导航被拒。结论见 `docs/superpowers/reviews/2026-10-28-dozer-browser-exposure.md`。

怎么跑(隔离,不碰你的 Dozer 数据):

```bash
mkdir -p /tmp/dozer-exposure /tmp/dozer-exposure-home
cp spike/dozer-browser-exposure/exposure.html /tmp/dozer-exposure/
# 临时把首页全局浏览器的初始网址改成复现页(不要提交):
#   crates/dozer-app/src/app/app.rs  home_browser: browser::State::with_initial_url("file:///tmp/dozer-exposure/exposure.html")
cargo build -p dozer-app -p dozerd
HOME=/tmp/dozer-exposure-home target/debug/dozer     # 首页右栏的全局浏览器会自动打开复现页
# 用 `orca computer get-app-state --app pid:<N> --window-index 0 --restore-window` 截图读日志(Orca 点不了 iced 控件,所以页面是自动跑的)
```

跑完关掉 Dozer,再关隔离 daemon:`lsof -U | grep dozer-exposure-home`。
