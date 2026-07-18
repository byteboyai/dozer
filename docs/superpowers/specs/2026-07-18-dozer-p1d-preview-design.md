# P1d 设计:左二资产预览 pane(wry WebView + Flyfish)

日期:2026-07-18 · 承接:规格 §3 需求 2、§7 左二、spike GO 报告(specs/2026-07-15-spike-report-webview.md)、P1c 收尾状态(main @ bf40a51)

## ⚠ 本设计代做的裁决(起草时用户暂离,均可否决)

| # | 裁决 | 备选(被放弃) |
|---|------|--------------|
| D1 | 文件入口 = **拖文件入预览区 + 地址栏输路径/URL**;文件树留 P1e | 连带最小文件树(侵入 P1e);只做网页基座(验不到预览主体) |
| D2 | **每 tab 一个 webview**,非激活 `set_visible(false)`;一期不设上限、不持久化 tabs | 单 webview 导航切换(丢 tab 状态:PDF 滚动位、网页表单) |
| D3 | Flyfish 资产 **vendor 进仓库**(从 npm tarball 提取 dist,锁版本),运行时自定义协议 serve,不起 HTTP 端口 | 首次运行联网下载(违背离线自托管);build.rs 下载(构建不可离线) |
| D4 | diff 预览 **只留 tab 类型接口,不实现**(渲染与数据流归验收闭环 P1f) | P1d 顺带做 diff(无交付数据流可接,空转) |
| D5 | 一期格式口径按规格裁剪执行:Markdown/图片/PDF/代码走 Flyfish;**若 full 包体积 > ~80MB,按 pipeline 精简到口径内格式** | 无脑全量 206 格式(包体积失控) |

## 1. 目标与验收口径

左二从占位变成真实的资产预览 pane:

1. **文件预览**:拖 `.md`/`.png`/`.pdf`/`.rs` 等文件进左二 → 开 tab,Flyfish 离线渲染;
2. **网页预览**:地址栏输入 `localhost:3000` 或任意 URL → 网页 tab(极简地址栏:URL 输入 + 刷新,无书签/历史/前进后退);
3. **tab 管理**:多 tab、切换、关闭;无 tab 时 iced 占位提示;
4. **布局契约**:webview 内容区恒为左二内容矩形,窗口 resize/pane 变化实时跟随;
5. 全程离线(除用户主动开的外部 URL);app 关闭预览 tabs 不持久化(一期 YAGNI)。

## 2. 架构

```
dozer-app
├── preview.rs        预览域状态机(纯数据,headless 全测)
│     PreviewPane { tabs: Vec<PreviewTab>, active: usize }
│     PreviewTab  { id: usize, kind: TabKind, title: String }
│     TabKind::File(PathBuf) | TabKind::Web { url: String, addr_input: String }
│     (TabKind::Diff 预留变体,P1f 实现)
├── assets.rs         Flyfish 静态资产表 + 自定义协议应答(纯函数,headless 全测)
│     dozer://flyfish/<path>  → vendored dist 字节
│     dozer://file/<abs-path> → 本地文件字节(仅白名单根:用户主动拖入/输入的路径)
├── workspace.rs      preview_pane():tab 栏 + 地址栏 + 占位(iced 绘制)
│     preview_content_bounds():左二内容矩形解析式换算(同终端 pane 手法)
└── main.rs           Runner::Ready 持 webviews: HashMap<usize, wry::WebView>
      消息分发环执行副作用:建/毁/显隐/set_bounds/导航(spike 约束 2:
      句柄只活在事件环,纯视图层不碰)
```

数据流:拖拽/地址栏 → `Message::PreviewOpen*` → `PreviewPane` 状态变更 → main.rs 对照状态与 `webviews` 差集,执行 wry 副作用(建 webview 用 `build_as_child`,文件 tab 载入 `dozer://flyfish/host.html?file=dozer://file/<path>`,网页 tab 直接 `with_url`)。

注:Flyfish 发行物是 JS 组件(`@file-viewer/web-full`,Web Components 形态),不是现成页面——vendored 资产里自带一张薄 host 页(`host.html`,仓库自写,挂载 `<file-viewer>` 组件并把 `?file=` 参数传给它),这是 Dozer 与 Flyfish 的唯一耦合点,也是 Preview Engine 抽象日后替换实现的接缝。

## 3. 关键决策依据

- **wry 0.55.1 子视图叠加**:spike 六项验收全过(渲染/resize/显隐/滚动点击/中文 IME/焦点),`build_as_child` 用法已验证;
- **自定义协议而非 localhost server**:无端口占用/防火墙弹窗/CORS 拉扯,离线语义天然成立;wry `with_custom_protocol` 三平台可用,架构不留 mac 专属绑定(符合"架构留门");
- **bounds 解析式换算**:iced 0.14 不暴露 widget 布局位置;P1c 的 `terminal_pane_pixel_size` 已验证"常量布局公式 + 向下取整"够用,预览内容区照搬同手法(左二 x = 左一宽,w = Fill 均分,y = 表头+tab 栏+地址栏高度);
- **webview 恒在 GPU 内容之上**(规格 §4 已裁决):左二是规则矩形,iced 不在其上画弹层;一期无 ⌘K,无隐藏诉求,但 `set_visible` 通路保留(spike ToggleGate 已验证)。

## 4. 错误处理

- 拖入不存在/不可读文件:tab 不建,RED 文案条(复用 daemon_error 展示手法);
- Flyfish 不支持的格式:Flyfish 自身兜底页呈现,Dozer 不逐格式 QA(规格裁剪原文);
- 网页加载失败:WKWebView 原生错误页,地址栏保留输入可改;
- 资产协议未命中(路径穿越/白名单外):返回 404 字节,拒绝 serve(`dozer://file` 仅允许用户显式打开过的路径集合)。

## 5. 测试策略

- headless:`PreviewPane` 状态机(开/关/切 tab、地址栏输入解析——路径 vs URL 判别)、协议应答纯函数(命中/404/路径穿越拒绝)、bounds 换算公式;
- 人工验收(T 末章清单):拖入 md/png/pdf/rs 四类文件、localhost 网页、外部 URL、tab 切换关闭、resize 跟随、终端与预览焦点互切、中文 IME 在网页表单内可用。

## 6. 显式不做(P1d 边界)

文件树入口(P1e)、diff 渲染(P1f)、会话审阅 tab(P1f)、"AI x 分钟前修改"状态条(P1e hook 事件)、tabs 持久化、⌘K、Office 长尾格式 QA、dbx(二期)。
