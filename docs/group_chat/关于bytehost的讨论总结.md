目前讨论已经形成一套偏保守、可落地的基座拆分原则，可以进入 spec 和第一阶段 plan。

## 一、核心原则

`bytehost` 的阶段性定义是：

> iced 桌面产品中，产品无关，且不属于 byteui、bytegit、独立面板或明确领域库的公共宿主能力。

采用“剩余即 host”的排除法，允许初期内部不够整洁，但要守住三条边界：

- 依赖方向固定：产品组合面板，面板依赖 host；host 不依赖 Dozer 或具体面板。
- host 公共 API 不出现 `TodoItem`、`Conversation`、`DeliveryStatus` 等业务类型。
- 临时面板特判必须集中登记，不能散落在 host 内部。

一期只建设一个 `bytehost` crate，不提前拆 `sdk`、`iced`、`testing`，也不为了未来需求预先设计大量 trait。等 Digger 接入后，根据真实差异再抽象。

## 二、当前归属结论

### 第一批明确共享、优先独立的面板

Digger 已确认会复用：

- Todo
- Conversation
- Agent
- Project 面板
- Files Tree

共享的是面板主体和通用领域能力。Dozer 的治理、交付、验收等语义仍留在 Dozer 产品层，不能随面板进入 host。

### 明确属于 host 的机制

- 应用和面板生命周期
- Workspace、Tab、栏位和布局容器
- Panel Registry
- 窗口、焦点、导航、快捷键及菜单分发
- 通用事件和 Toast 机制
- 通用 Overlay / Surface 承载与几何同步
- 文件选择、拖放等系统接入
- 设置的注册、持久化和展示机制

归属应按“机制”判断，不能把现有 `app/`、`platform/`、`extensions/` 整个目录直接搬入 host。

### 明确留在 Dozer 产品层

- 治理、交付和验收语义
- 默认面板组合与默认布局
- Dozer 品牌、主题内容和产品设置
- 具体面板的分支及专属 overlay
- `delivery`
- Dozer composition root
- 目前的 Preview 业务

### 暂存或待审计的公共服务

- Project context：项目打开、关闭、当前项目
- `secrets`
- `external_apps`
- `capabilities`
- 跨面板协调状态
- `conversation`、`transcript` 等领域模型的最终归属
- Git watch、Git accounts 等应进入 bytegit 还是独立服务

这些可以暂时寄存在 host，但不意味着它们永久属于 host 核心。

## 三、Preview 与 Digger Writing

当前采取保守方案：

- Dozer 现有 Preview 暂不作为共享面板拆出。
- CodeMirror、文件类型路由、HTML/JSON 等查看器继续留在 Dozer。
- 通用 WebView/Surface 的生命周期、几何、焦点和层级机制可以进入 host。
- Digger 新建独立 Writing 面板，面向富文本创作、文档结构和内容组织，不把它设计成 Preview 的变体。

Files Tree 应与 Preview 解耦：

```text
Files Tree → 通用打开命令/目标注册 → 产品决定处理者
```

Dozer 路由到 Preview，Digger 路由到 Writing。具体协议仍需在审计后确定。

## 四、暂未完全裁决的问题

需要在 spec 中明确标为未决：

- Agent 的面板主体和会话运行服务具体怎么切。
- Project context 进入 host 的范围。
- Files Tree 的“打开目标”协议。
- `secrets`、`external_apps`、`capabilities` 的归属。
- 移除 `PanelKind` 后，面板默认栏位由注册信息还是产品 composition root 决定。
- Terminal 是否纳入当前共享范围。架构上更像独立面板，但 Digger 的真实需求尚未确认。

## 五、下一步文档范围

现在可以转为两个待办：

1. 编写一份 bytehost 边界与拆分 spec，记录原则、四类归属、依赖规则、验收标准、兼容债务和未决项。
2. 只编写第一阶段 H0 plan，先做只读审计与迁移清单，包括：
   - 各候选模块对 `App`、`Workspace` 和其他面板的依赖统计；
   - 顶层共享模型及 host 反向调用的审计；
   - 面板边界门禁设计；
   - `platform/` 下各面板专属 overlay 的迁移清单。

H1 以后的实施计划等 H0 给出真实依赖结果后再写，避免把未经验证的归属假设固化进路线图。
