# Dozer Sketch MVP

> Dozer Workspace 内置的 AI-Native UI Sketch / Wireframe 工具。
>
> 核心定位：**UI Intent Editor，而不是 Design Tool。**

## 1. 产品定位

Dozer Sketch 用于让人类快速表达产品/UI 意图，并将这些意图以结构化数据保存，使 AI Agent 获得比单纯视觉理解更丰富的 UI 信息。

核心理念：

> **Human defines intent → Sketch structures intent → Agent executes intent**

Sketch 不追求替代 Figma，也不追求高保真设计。

它应该比 Figma、Sketch、Adobe XD 更简单，比 Excalidraw 更结构化，比 Balsamiq 更 AI-friendly。

---

# 2. 核心目标

## 2.1 人类侧

用户应该能够在几十秒内画出一个简单页面：

- 页面布局
- 标题
- 文本
- 输入框
- 按钮
- 图片占位
- 卡片
- 列表
- 简单区域关系

不需要学习复杂设计工具。

## 2.2 AI 侧

AI Agent 应该能够直接读取 Sketch 的结构化信息：

- 页面
- 元素
- 元素类型
- 层级
- 几何位置
- 文本
- 语义
- 页面之间的关系
- 用户意图
- 交互行为

AI 不应该依赖 OCR 或 Computer Vision 才能理解原型。

## 2.3 Dozer 侧

Sketch 应成为 Dozer Agent Workspace 中的一个可读写 UI Artifact。

目标工作流：

```text
Human
  ↓
Sketch
  ↓
UI Intent / UI AST
  ↓
Agent
  ↓
Code
  ↓
Browser Preview
  ↓
Human Feedback
  ↓
Sketch / Agent
```

---

# 3. 非目标

MVP 阶段明确不实现：

- 高保真 UI 设计
- 完整 Figma 功能
- Auto Layout
- Constraint System
- Design Token
- Component Variant
- Responsive Design
- Prototype Animation
- 复杂 SVG 编辑
- 复杂路径编辑
- Photoshop 类图像编辑
- 多人实时协作
- 云端设计系统
- 插件生态
- UI Design System 管理

原则：

> **如果一个功能主要服务于“把页面设计得更漂亮”，MVP 暂时不要实现。**

---

# 4. 核心产品模型

Sketch 不应该被设计成传统 Design File。

它本质上是一个：

```text
UI Intent Graph
```

或者：

```text
UI AST
```

视觉信息只是 UI AST 的一个表现层。

每个节点至少包含：

```text
Visual
Semantic
Intent
Relationship
```

---

# 5. Sketch 文件结构

MVP 使用单文件格式：

```text
*.sketch
```

推荐使用 JSON 格式，便于：

- AI 读取
- Git diff
- Agent 修改
- 调试
- 人工检查
- 未来版本迁移

示例：

```json
{
  "version": 1,
  "document": {
    "name": "Login"
  },
  "pages": [],
  "metadata": {}
}
```

---

# 6. Page

一个 Sketch Document 可以包含多个 Page。

Page：

```json
{
  "id": "page_login",
  "name": "Login",
  "width": 1440,
  "height": 900,
  "root": []
}
```

Page 用于表达一个独立 UI Screen。

例如：

```text
Login
Dashboard
Project
Agent
Settings
```

---

# 7. Node

所有 UI 元素统一抽象成 Node。

基础结构：

```json
{
  "id": "btn_login",
  "type": "button",
  "name": "Login Button",
  "text": "Login",
  "bounds": {
    "x": 420,
    "y": 360,
    "width": 120,
    "height": 40
  },
  "style": {},
  "children": [],
  "actions": [],
  "metadata": {}
}
```

---

# 8. MVP Node Types

第一阶段只支持：

```text
frame
group
text
button
input
image
shape
```

## 8.1 Frame

用于：

- Page 内区域
- Header
- Sidebar
- Content
- Modal
- Card
- Section

示例：

```json
{
  "type": "frame",
  "name": "Header"
}
```

---

## 8.2 Group

用于将多个 Node 组织成逻辑结构。

例如：

```text
Login Form
├── Email Input
├── Password Input
└── Login Button
```

---

## 8.3 Text

用于：

- 标题
- 标签
- 描述
- 普通文本

属性：

```text
text
font_size
font_weight
alignment
```

MVP 不需要复杂 Typography。

---

## 8.4 Button

属性：

```text
text
variant
actions
```

例如：

```json
{
  "type": "button",
  "text": "Login",
  "actions": [
    {
      "event": "click",
      "action": "navigate",
      "target": "dashboard"
    }
  ]
}
```

---

## 8.5 Input

属性：

```text
label
placeholder
input_type
value
```

例如：

```json
{
  "type": "input",
  "name": "Email",
  "placeholder": "Email",
  "input_type": "email"
}
```

---

## 8.6 Image

MVP 主要作为图片占位符。

属性：

```text
src
alt
fit
```

如果没有实际图片：

```text
Image Placeholder
```

即可。

---

## 8.7 Shape

用于：

- Rectangle
- Circle
- Divider

MVP 不需要复杂 Vector Path。

---

# 9. Semantic Layer

这是 Dozer Sketch 最重要的能力。

Node 不应该只有：

```text
x
y
width
height
```

还应该拥有：

```text
semantic_type
role
meaning
```

例如：

```json
{
  "type": "shape",
  "semantic": {
    "role": "card"
  }
}
```

或者：

```json
{
  "type": "frame",
  "semantic": {
    "role": "navigation"
  }
}
```

MVP 支持常见 Semantic Role：

```text
page
header
footer
navigation
sidebar
content
section
card
form
list
table
modal
dialog
toolbar
search
```

Semantic Role 可以为空。

---

# 10. Intent Layer

Intent 是 Dozer Sketch 区别于传统 Wireframe 工具的核心。

Node 可以表达用户意图：

```json
{
  "intent": {
    "purpose": "login user"
  }
}
```

或者：

```json
{
  "intent": {
    "purpose": "search projects"
  }
}
```

MVP 不要求复杂 NLP Schema。

只需要提供一个可扩展结构：

```json
{
  "intent": {
    "purpose": "...",
    "description": "...",
    "metadata": {}
  }
}
```

---

# 11. Interaction Layer

Node 可以定义 Action。

基础结构：

```json
{
  "event": "click",
  "action": "navigate",
  "target": "dashboard"
}
```

MVP Action：

```text
navigate
open
close
submit
toggle
custom
```

Event：

```text
click
change
submit
```

---

# 12. Page Relationship

Page 之间允许建立关系。

例如：

```text
Login
  ↓
Dashboard
  ↓
Project
  ↓
Agent
```

保存：

```json
{
  "source": "page_login",
  "event": "click",
  "node": "btn_login",
  "action": "navigate",
  "target": "page_dashboard"
}
```

最终可以形成：

```text
Screen Graph
```

---

# 13. Visual Layer

Visual Layer 只负责表现。

MVP 支持：

```text
position
size
font
font_size
font_weight
border
background
radius
opacity
alignment
```

不需要追求 CSS 级精度。

原则：

> Visual information should be sufficient for humans to understand the wireframe, but should not dominate the data model.

---

# 14. UI 风格

默认采用 Wireframe 风格。

特点：

- 简洁
- 黑白 / 灰度
- 低视觉噪音
- 轻量手绘感
- 不追求高保真
- 元素边界清晰

目的：

让用户明确：

> 这是产品结构，而不是最终设计稿。

---

# 15. Canvas

Canvas MVP：

```text
无限画布
Zoom
Pan
Select
Move
Resize
Delete
```

支持：

```text
Mouse
Trackpad
Keyboard
```

快捷键：

```text
V       Select
F       Frame
T       Text
B       Button
I       Input
R       Shape
G       Group
Delete  Delete
Cmd/Ctrl+Z Undo
Cmd/Ctrl+Shift+Z Redo
Space   Pan
```

快捷键允许后续调整。

---

# 16. Toolbar

MVP Toolbar：

```text
Select
Frame
Text
Button
Input
Image
Shape
Group
```

不要加入大量高级工具。

---

# 17. Selection

选中 Node 后显示：

```text
Bounding Box
Resize Handles
```

右侧显示最小属性面板：

```text
Name
Type
Text
Semantic Role
Intent
```

视觉属性可以提供：

```text
Width
Height
Font Size
```

MVP 不需要复杂 Inspector。

---

# 18. AI 操作

Sketch 必须允许 Agent 对 Document 进行读写。

至少提供以下能力：

## Read

```text
get_document
get_page
get_node
get_tree
get_screen_graph
```

## Modify

```text
create_node
update_node
delete_node
move_node
resize_node
group_nodes
ungroup_nodes
```

## Semantic

```text
set_semantic
set_intent
add_action
remove_action
```

## Page

```text
create_page
rename_page
delete_page
connect_pages
```

---

# 19. AI Command 示例

用户：

> 给 Login 页面增加一个忘记密码链接。

Agent 应该可以直接执行：

```text
create_node
type = text
text = "Forgot Password?"
```

并设置：

```text
semantic.role = link
intent.purpose = "recover password"
```

---

# 20. AI 修改示例

用户：

> 把 Login 页面改成左右两栏。

Agent 应该能够读取当前 AST：

```text
Login
├── Email
├── Password
└── Login Button
```

然后修改成：

```text
Login
├── Left Panel
│   └── Image
│
└── Right Panel
    ├── Email
    ├── Password
    └── Login Button
```

不应该通过“重新生成一张图片”实现。

---

# 21. AI Export

Sketch 应提供一个稳定的 AI-readable representation。

例如：

```text
Page: Login

Layout:
  Two-column

Left:
  Image

Right:
  Form
    Input: Email
    Input: Password
    Button: Login

Interactions:
  Login Button -> Dashboard
```

这个 Representation 用于：

- Agent Context
- LLM Prompt
- Debug
- Export
- Code Generation

---

# 22. AI Context 优先级

向 Agent 提供信息时优先：

```text
Intent
Semantic
Structure
Interaction
Text
Geometry
Visual Style
```

而不是：

```text
Screenshot
```

Screenshot 只作为辅助信息。

---

# 23. AI + Screenshot 双模式

Sketch 可以同时提供：

```text
Structured Context
+
Rendered Screenshot
```

Agent 获取：

```text
1. UI AST
2. Semantic Tree
3. Screen Graph
4. Screenshot
```

其中：

```text
UI AST = Primary Source of Truth
Screenshot = Visual Reference
```

---

# 24. Undo / Redo

MVP 必须支持：

```text
Undo
Redo
```

建议基于 Command / Operation History 实现。

每次修改 Document：

```text
Operation
```

例如：

```text
CreateNode
MoveNode
ResizeNode
DeleteNode
UpdateProperty
```

这样未来 AI 修改也可以进入同一套 Undo/Redo 系统。

---

# 25. AI 修改必须可追踪

AI Agent 修改 Sketch 时，应该能够知道：

```text
Who:
AI Agent

What:
Created Button

Why:
User requested login action

When:
timestamp
```

MVP 可以先保留：

```json
{
  "source": "agent",
  "agent": "coder",
  "reason": "..."
}
```

未来可以发展成完整 Agent Activity Log。

---

# 26. Rust 实现建议

Dozer 当前采用 Rust + Iced，因此 Sketch MVP 推荐：

```text
Rust
├── iced
├── serde
├── serde_json
├── uuid
└── anyhow / thiserror
```

核心架构：

```text
dozer-sketch
│
├── model
│   ├── document
│   ├── page
│   ├── node
│   ├── semantic
│   ├── intent
│   └── action
│
├── canvas
│   ├── renderer
│   ├── selection
│   ├── interaction
│   └── transform
│
├── editor
│   ├── commands
│   ├── history
│   └── clipboard
│
├── persistence
│   ├── serializer
│   └── migration
│
└── ai
    ├── reader
    ├── writer
    └── context
```

---

# 27. Core Data Model

推荐最终统一成：

```rust
struct SketchDocument {
    version: u32,
    metadata: DocumentMetadata,
    pages: Vec<Page>,
}

struct Page {
    id: NodeId,
    name: String,
    bounds: Bounds,
    children: Vec<Node>,
}

struct Node {
    id: NodeId,
    node_type: NodeType,
    name: String,
    bounds: Bounds,
    style: Style,
    content: Content,
    semantic: Option<Semantic>,
    intent: Option<Intent>,
    actions: Vec<Action>,
    children: Vec<Node>,
    metadata: Metadata,
}
```

其中：

```rust
enum NodeType {
    Frame,
    Group,
    Text,
    Button,
    Input,
    Image,
    Shape,
}
```

---

# 28. Architecture Principle

核心架构必须做到：

```text
Model
  ↓
Commands
  ↓
State
  ↓
Renderer
```

Renderer 不应该成为数据源。

不要：

```text
Canvas → 直接修改 UI 状态
```

而应该：

```text
User
 ↓
Command
 ↓
Document
 ↓
Renderer
```

AI 也是一样：

```text
AI
 ↓
Command
 ↓
Document
 ↓
Renderer
```

这样 Human 和 Agent 使用同一套修改机制。

---

# 29. AI Tool API

建议定义稳定的内部 Tool Protocol：

```text
sketch.get_document()
sketch.get_page(page_id)
sketch.get_node(node_id)

sketch.create_node(...)
sketch.update_node(...)
sketch.delete_node(...)

sketch.move_node(...)
sketch.resize_node(...)

sketch.set_semantic(...)
sketch.set_intent(...)
sketch.add_action(...)

sketch.create_page(...)
sketch.delete_page(...)
sketch.connect_pages(...)
```

未来可以直接映射到 MCP Tool。

---

# 30. MVP 验收标准

## Canvas

- [ ] 可以创建 Page
- [ ] 可以放置 Node
- [ ] 可以选择 Node
- [ ] 可以移动 Node
- [ ] 可以调整大小
- [ ] 可以删除 Node
- [ ] 支持 Zoom / Pan
- [ ] 支持 Undo / Redo

## Node

- [ ] Frame
- [ ] Group
- [ ] Text
- [ ] Button
- [ ] Input
- [ ] Image
- [ ] Shape

## Semantic

- [ ] Node 可以设置 Semantic Role
- [ ] Semantic 信息可以保存
- [ ] Semantic 信息可以被 AI 读取

## Intent

- [ ] Node 可以设置 Intent
- [ ] Intent 可以保存
- [ ] Intent 可以被 AI 读取

## Interaction

- [ ] Button 可以定义 Click Action
- [ ] Page 可以建立 Navigate Relationship
- [ ] 可以生成 Screen Graph

## Persistence

- [ ] `.sketch` 文件可以保存
- [ ] `.sketch` 文件可以加载
- [ ] JSON 格式稳定
- [ ] version 字段存在
- [ ] 支持基础 migration 机制

## AI

- [ ] AI 可以读取完整 Document
- [ ] AI 可以读取 Page
- [ ] AI 可以读取 Node Tree
- [ ] AI 可以创建 Node
- [ ] AI 可以修改 Node
- [ ] AI 可以删除 Node
- [ ] AI 可以设置 Semantic
- [ ] AI 可以设置 Intent
- [ ] AI 可以建立 Page Relationship

---

# 31. MVP 完成后的典型流程

用户打开 Dozer：

```text
New Project
   ↓
New Sketch
   ↓
画一个 Login 页面
   ↓
选择 Login Button
   ↓
设置 Semantic = action
   ↓
设置 Intent = login user
   ↓
连接 Dashboard
```

此时 Agent 可以直接读取：

```text
Page: Login

Elements:
  Email Input
  Password Input
  Login Button

Intent:
  Authenticate user

Interaction:
  Login Button
    → Dashboard
```

然后用户：

```text
"根据这个原型实现页面。"
```

Coder Agent 直接开始工作。

---

# 32. 后续版本方向

MVP 完成后，再考虑：

### V2

```text
Card
Table
List
Modal
Tabs
Navigation
Avatar
Checkbox
Radio
Select
```

### V3

```text
Component
Reusable Component
Design Token
Theme
Responsive
```

### V4

```text
AI Generate UI
AI Refactor UI
AI Analyze UX
AI Generate Code
AI Generate PRD
AI Generate User Flow
```

### V5

```text
Multi-Agent Collaborative Design
```

例如：

```text
Designer Agent
      ↓
Sketch
      ↓
Coder Agent
      ↓
Browser
      ↓
QA Agent
      ↓
Sketch Feedback
```

---

# 33. 最重要的产品原则

Dozer Sketch 必须遵守以下原则：

### 1. Simple First

> 如果一个功能不能显著提高原型表达能力，就不要加入。

### 2. Intent First

> Sketch 的核心资产不是像素，而是用户意图。

### 3. Semantic First

> AI 应该读取结构，而不是猜测结构。

### 4. Human Fast

> 人类应该可以在几十秒内完成一个低保真原型。

### 5. AI Native

> AI 不只是 Sketch 的使用者，也应该是 Sketch 的编辑者。

### 6. Code Friendly

> `.sketch` 必须是机器可读、可 diff、可版本控制的格式。

### 7. Dozer Native

> Sketch 不是独立设计软件，而是 Dozer Agent Workspace 的 UI Intent Layer。

---

# 34. 最终定位

Dozer Sketch 不应该成为：

> “Rust 版 Figma”

也不应该只是：

> “Rust 版 Balsamiq”

而应该成为：

> **一个面向 AI Agent 的 UI Intent Editor。**

最终形成：

```text
Human
  │
  │  快速表达想法
  ▼
┌──────────────────┐
│   Dozer Sketch   │
│                  │
│ Visual           │
│ Semantic         │
│ Intent           │
│ Interaction      │
└────────┬─────────┘
         │
         │ UI AST
         ▼
┌──────────────────┐
│   Dozer Agents   │
├──────────────────┤
│ Product Agent    │
│ Designer Agent   │
│ Coder Agent      │
│ QA Agent         │
└────────┬─────────┘
         │
         ▼
       Code
         │
         ▼
     Browser
         │
         ▼
      Feedback
         │
         └──────────→ Sketch
```

**核心一句话：**

> **Dozer Sketch 让人类负责“画出想法”，让 AI 获得“想法的结构”。**