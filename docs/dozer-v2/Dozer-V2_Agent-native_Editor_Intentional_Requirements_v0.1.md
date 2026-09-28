# Dozer Agent-native Editor 意向性需求文档

**文档性质：** 产品意向 / 顶层需求\
**项目：** Dozer\
**模块：** Agent-native Editor\
**状态：** Draft\
**版本：** v0.1

------------------------------------------------------------------------

## 1. 背景

Dozer 的文件能力不应仅定位为传统意义上的 File
Viewer，也不以实现一个完整的 Office 套件为目标。

在传统编辑器中，Human 是主要编辑主体，软件提供查看和编辑工具；AI
通常只是附加能力，例如问答、生成文本或辅助修改。

Dozer 希望探索另一种工作模式：

> **Human 负责阅读、判断、提出修改意见和验收结果；Agent
> 成为主要的文件编辑执行者。**

Human
不需要学习和操作大量复杂的文件编辑功能，而是直接针对自己正在查看的内容提出要求，由
Agent 对文件进行精确修改，并将修改结果立即反馈到 Human 当前工作视野中。

因此，Dozer 的文件系统需要从：

> **File Viewer + Agent**

逐步演进为：

> **Agent-native 通用文件编辑器（Agent-native Editor）**

其核心不是"让 AI 能打开文件"，而是建立：

> **Human 看见 → Human 指向 → Agent 理解 → Agent 修改 → Human 原位验证**

的完整闭环。

------------------------------------------------------------------------

## 2. 产品定义

Dozer Agent-native Editor 的核心模型定义为：

> **Agent-native Editor = Viewer + Shared Viewport + Typed Selection +
> Context Scope + Mutation Engine + Change Feedback**

其中：

-   **Viewer**：Human 可以查看文件；
-   **Shared Viewport**：Agent 知道 Human 当前正在查看什么位置；
-   **Typed Selection**：Human 可以精确选择文件内部对象并发送给 Agent；
-   **Context Scope**：Human 可以明确控制 Agent
    当前可以使用和操作的文件范围；
-   **Mutation Engine**：Agent
    可以对不同类型文件执行结构化、精确的修改；
-   **Change Feedback**：Agent 修改后，Human
    可以立即看到结果并进行审阅。

工程原则：

> **Every agent edit is anchored, scoped, transactional, traceable, and
> reversible.**

即：

> **Agent
> 的每一次修改，都必须可定位、有边界、具备事务性、可追溯、可撤销。**

------------------------------------------------------------------------

## 3. 核心设计原则

### 3.1 Agent 是主要编辑主体

Dozer 不以复刻 Word、Excel、Photoshop
等专业软件的全部人工编辑能力为目标。

主要交互方式：

``` text
Human 阅读文件
      ↓
发现问题
      ↓
选择内容 / 指出位置
      ↓
提出修改意见
      ↓
Agent 执行修改
      ↓
文件立即刷新
      ↓
Human 原位审阅
```

Human 仍然拥有最终控制权，但 Agent 承担主要编辑操作。

因此，应优先建设：

-   文件预览能力；
-   精确定位能力；
-   Agent 修改能力；
-   修改反馈能力；

而不是优先建设复杂的传统人工编辑 UI。

------------------------------------------------------------------------

## 4. Viewer ------ 通用文件预览

Dozer 应尽可能扩大文件预览覆盖范围。

目标不是让所有格式都达到专业编辑器级别，而是尽可能让 Human 可以在 Dozer
内部直接阅读和审阅文件。

优先支持：

### 文本与代码

-   TXT
-   Markdown
-   JSON
-   YAML
-   TOML
-   XML
-   CSV
-   各类源代码文件

### 表格

-   XLSX
-   XLS
-   CSV
-   TSV
-   ODS（可选）

优先使用 Dozer 自身的 `dozer-tabular`。

### 文档

-   DOCX
-   RTF
-   ODT（后续）

### PDF

支持阅读、页定位、文本选择、表格/图片区域定位以及 Agent Context。

### 图片

支持 PNG、JPEG、WebP、SVG 等常见格式，后续按实际需求扩展。

------------------------------------------------------------------------

## 5. Shared Viewport ------ Human 与 Agent 共享视野

Shared Viewport 是 Agent-native Editor 的核心能力之一。

Agent 不仅需要知道 Human 打开了哪个文件，还应该知道 Human
此刻正在看文件中的什么位置。

例如：

``` text
文本：
main.rs
line 183–227

表格：
valuation.xlsx
Sheet1
B17:H42

DOCX：
report.docx
paragraph 37–43

PDF：
annual_report.pdf
page 27

图片：
chart.png
viewport x/y/w/h
```

Viewport 应作为 Dozer Agent Runtime 的一等状态，而不是简单作为聊天附件。

Human 可以直接说：

> "这里的数据为什么不对？"

Agent 应能够理解"这里"所指的位置。

------------------------------------------------------------------------

## 6. 双向 Shared Viewport

Shared Viewport 不应仅支持 Human → Agent，也应该支持 Agent → Human。

Agent 可以主动将 Human 导航至发现的问题：

``` text
Agent
  ↓
focus(report.docx, anchor)
  ↓
Viewer scroll
  ↓
highlight
```

形成：

``` text
Human → “看看这里”
Agent → “问题实际在这里”
Human → “修改它”
Agent → 修改
Viewer → 原位刷新
Human → 验收
```

Viewer 应向 Agent Runtime 提供标准化的
`focus`、`scroll_to`、`highlight`、`reveal` 等能力。

------------------------------------------------------------------------

## 7. Typed Selection ------ 文件内部精确选择

Human 应能够在 Viewer 内选择文件内部内容，并通过右键等方式 **Send to
Agent**。

Selection 根据文件类型定义。

### 7.1 Text Selection

``` text
file
line_start
column_start
line_end
column_end
selected_text
```

### 7.2 Spreadsheet Selection

``` text
file
sheet
range
```

例如：

``` text
valuation.xlsx
Sheet1
B17:F32
```

### 7.3 Document Selection

DOCX 不宜仅依赖视觉上的"行号"，建议：

``` text
file
block_id
paragraph_id
text_offset
selected_text
```

### 7.4 PDF Selection

``` text
file
page
bbox
text_range
selected_text
```

### 7.5 Image Selection

``` text
file
x
y
width
height
```

Human 可以框选图片区域后直接 Ask Agent about this region 或 Edit this
region。

------------------------------------------------------------------------

## 8. 从 Typed Selection 演进至 Typed Object

长期来看，Agent 操作的对象不应该只是坐标，而应该逐渐成为结构化对象。

### DOCX

-   paragraph
-   heading
-   table
-   table_cell
-   image
-   comment

### Spreadsheet

-   cell
-   range
-   row
-   column
-   table
-   chart
-   formula

### Code

-   line
-   function
-   class
-   symbol
-   AST node

### PDF

-   text_block
-   table
-   figure
-   annotation

### Image

-   region
-   object
-   layer

长期目标：

> **Typed Selection → Typed Object Reference**

坐标负责底层定位，Object 成为 Human 与 Agent 交流的主要语义单位。

------------------------------------------------------------------------

## 9. Stable Anchor ------ 稳定定位

单纯依赖 `line 37`、`paragraph 42`、`page 17` 不足以支撑可靠的 Agent
编辑，因为文件修改后位置可能发生变化。

概念模型：

``` text
Anchor {
    resource_id
    object_id
    position
    content_hash
    surrounding_context
}
```

不同文件 Adapter 使用不同 Anchor 策略：

-   **Text**：line/column + content fingerprint
-   **DOCX**：paragraph/block ID + text offset + content fingerprint
-   **Spreadsheet**：sheet + cell/range
-   **PDF**：page + bbox + text fingerprint

Stable Anchor 应支持修改前定位、修改后重新定位、Change History
定位、Agent → Human 导航以及 Conflict Detection。

------------------------------------------------------------------------

## 10. Context Scope ------ Agent 工作上下文

Agent Terminal 下方应存在明确的"正在使用 / 操作的资源列表"：

``` text
Agent Context
────────────────────
📁 research/
📄 report.docx
📊 valuation.xlsx
📄 assumptions.md
```

Human 可以通过文件树右键 Add to Agent Context。目录代表其下全部文件。

应明确区分：

``` text
Context Scope
Agent 可以使用哪些资源

Live View
Human 当前正在查看什么

Selection
Human 当前明确指出了什么
```

三者语义不能混淆。

------------------------------------------------------------------------

## 11. Context Permission

加入 Context 不应自动意味着 Agent 可以修改。

例如：

``` text
📁 research/          Read
📊 raw_data.xlsx      Read
📊 model.xlsx         Read / Write
📄 report.docx        Read / Write
🔒 source.pdf         Read Only
```

长期可以形成：

``` text
Visible
   ↓
Readable
   ↓
Editable
   ↓
Executable
```

不同级别的 Agent 能力边界。

------------------------------------------------------------------------

## 12. Mutation Engine ------ Agent 统一编辑层

Agent 不应直接操作 DOCX XML、XLSX OOXML 等底层文件格式。

统一流程：

``` text
LLM
 ↓
Intent
 ↓
Mutation Plan
 ↓
Mutation Engine
 ↓
File Adapter
 ↓
Physical File
```

Mutation Engine 提供统一的高层操作语义，例如：

``` text
replace()
insert()
delete()
rewrite()
move()
```

不同文件类型由对应 Adapter 实现：

``` text
TextAdapter
DocxAdapter
SpreadsheetAdapter
ImageAdapter
```

------------------------------------------------------------------------

## 13. Intent 与 Operation 分离

Agent 的自然语言意图：

> "把这段话改得更加专业。"

属于 Intent。

实际执行：

``` text
replace_text(
    anchor,
    expected_hash,
    new_text
)
```

属于 Operation。

LLM 负责理解 Human 意图和生成 Mutation Plan。

Dozer Mutation Engine 负责：

-   权限验证；
-   Anchor 校验；
-   Conflict Detection；
-   Mutation 执行；
-   文件格式处理；
-   Transaction；
-   History；
-   Rollback。

------------------------------------------------------------------------

## 14. 局部修改与整篇重写

Mutation Engine 必须同时支持精确局部修改和 Whole-file Rewrite。

局部修改例如：

``` text
replace paragraph
replace cells
insert row
delete block
replace image region
```

局部修改应作为 Agent 编辑的默认模式。

对于 Markdown、小型文本、Human
明确要求重新生成或大范围结构重组，可以支持：

``` text
rewrite(resource)
```

但整篇覆盖不应成为所有文件类型的默认编辑方式。

------------------------------------------------------------------------

## 15. Transaction ------ 修改事务

Agent 的修改应通过 Transaction 执行：

``` text
Agent Intent
      ↓
Mutation Plan
      ↓
Transaction
 ├─ update XLSX
 ├─ update DOCX
 └─ update chart
      ↓
Validate
      ↓
Commit
```

如果其中一步失败则 Rollback。

长期支持：

-   Single-file Transaction；
-   Multi-operation Transaction；
-   Multi-file Transaction。

------------------------------------------------------------------------

## 16. Conflict Detection

Agent 从读取文件到真正执行修改之间，Human 或其他 Agent
可能已经修改文件。

Mutation 应携带：

``` text
expected_version
expected_hash
```

如果：

``` text
current_hash != expected_hash
```

则产生 `MutationConflict`。

Agent 应重新读取相关内容，而不是直接覆盖最新文件。

------------------------------------------------------------------------

## 17. Change Feedback ------ 修改立即可见

Agent 修改完成后，Human 不应重新寻找修改位置。

系统应：

``` text
Mutation
   ↓
refresh affected region
   ↓
resolve anchor
   ↓
scroll_to(anchor)
   ↓
highlight changed content
```

例如 Human 正在查看 `Sheet1!D173`，Agent 修改 `D173:F181`，完成后 Viewer
应仍然停留在该区域，并短暂 Highlight。

核心原则：

> **保持 Human Attention Continuity。**

"立即可见"不仅意味着刷新速度快，更意味着 Human 的注意力不会因为 Agent
修改而丢失。

------------------------------------------------------------------------

## 18. Change History

Agent 修改应产生结构化 History：

``` text
11:16 report.docx
修改收入预测说明
3 paragraphs changed

11:14 valuation.xlsx
更新 Sheet1!D17:F32
48 cells changed
```

Human 可以执行：

-   Locate
-   Diff
-   Revert
-   Ask Agent

其中 Locate 应直接将 Viewer 导航回对应修改位置。

------------------------------------------------------------------------

## 19. Provenance ------ 修改来源与原因

Agent History 不应只是普通 Undo Log。

每次修改应记录：

``` text
Agent
Task
Source
Target
Mutation
Before
After
Reason
Session
Timestamp
```

例如：

``` text
Change #184

Task:
按照 valuation.xlsx 更新报告中的估值

Source:
valuation.xlsx
Sheet1!D17:F21

Target:
report.docx
paragraph #182

Before:
预计收入 32.4 亿元

After:
预计收入 35.7 亿元

Reason:
Source data updated
```

这使 Change History 同时成为 **Agent Audit Trail**。

------------------------------------------------------------------------

## 20. Review 模式

Human 可以控制 Agent 修改后的审阅方式。

### Quiet Mode

``` text
Agent 修改
→ Viewer 自动刷新
→ Highlight
→ History 记录
```

不弹出额外窗口。

### Review Mode

修改完成后自动打开：

``` text
Before │ After
```

Human 可以：

-   Accept
-   Revert
-   Ask Agent
-   Locate

是否自动打开 Change Review 由 Human 控制。

------------------------------------------------------------------------

## 21. 总体架构

建议形成三层体系：

``` text
             Dozer Agent-native Editor

┌──────────────────────────────────────┐
│          Human Interaction           │
│                                      │
│ Viewer ─── Shared Viewport           │
│    │            │                    │
│ Typed Selection ─ Context Scope      │
│    │                                 │
│ Change Feedback                      │
└─────────────────┬────────────────────┘
                  │
┌─────────────────▼────────────────────┐
│          Agent Editing Layer         │
│                                      │
│ Typed Object / Stable Anchor         │
│              ↓                       │
│ Intent → Mutation Plan               │
│              ↓                       │
│        Mutation Engine               │
└─────────────────┬────────────────────┘
                  │
┌─────────────────▼────────────────────┐
│         Safety & Integrity           │
│                                      │
│ Permission                           │
│ Transaction                          │
│ Conflict Detection                   │
│ Validation                           │
│ Provenance / History                 │
│ Rollback                             │
└──────────────────────────────────────┘
```

------------------------------------------------------------------------

## 22. Viewer / Adapter 架构

不同文件 Viewer 不需要采用相同技术栈。

Dozer 应统一的是协议，而不是 UI 实现技术。

``` text
                Agent-native Editor Protocol

                         │
          ┌──────────────┼──────────────┐
          │              │              │
     Native Viewer   Web Viewer    External Engine
          │              │              │
   dozer-tabular     DOCX Editor       ...
   JSON Tree         CodeMirror
                     PDF Viewer
```

每个 Viewer 尽可能实现统一能力：

``` text
Viewport
Selection
Typed Object
Anchor
Mutation
Focus
Highlight
Refresh
History
```

------------------------------------------------------------------------

## 23. 初期文件策略

### Spreadsheet

继续使用 **dozer-tabular**。

重点发展：

-   大文件性能；
-   range selection；
-   cell/range anchor；
-   Agent Mutation API；
-   change highlight；
-   structured data operations。

Dozer 不以重新实现完整 Excel 为目标。

### DOCX

重点寻找：

> **轻量级 DOCX Viewer / Editor + Agent Mutation Adapter**

优先要求：

1.  高质量预览；
2.  Selection；
3.  Stable Anchor；
4.  局部 Mutation；
5.  Refresh；
6.  Focus / Highlight。

### Text / Code

利用成熟编辑组件实现 Viewport、Selection、line/column anchor、Diff 和
Agent patch。

### JSON

以 **高性能 Tree Viewer + Typed Object** 为核心。JSON Path 可以天然成为
Agent Object Locator。

### PDF

初期主要定位为 **Viewer + Selection + Context**。PDF
原文件修改能力可以后置。

### Image

初期支持 Viewer、Region Selection、Agent Context
和修改后刷新，后续逐步发展对象级操作。

------------------------------------------------------------------------

## 24. MVP 建议

第一阶段不需要一次性完成所有文件格式。

优先验证：

``` text
打开文件
   ↓
Human 查看
   ↓
Human Selection
   ↓
Send to Agent
   ↓
Agent 精确修改
   ↓
Viewer 原位刷新
   ↓
Highlight
   ↓
Human Review
```

首批可以选择：

``` text
Text / Markdown
        +
dozer-tabular
        +
DOCX
```

三类文件分别代表纯文本、结构化数据和富文本文档。

如果三者能够通过统一协议完成相同的 Human-Agent 编辑闭环，则 Agent-native
Editor 的基础架构基本得到验证。

------------------------------------------------------------------------

## 25. 暂不追求

早期不应以以下目标作为核心：

-   完整替代 Microsoft Word；
-   完整替代 Microsoft Excel；
-   完整替代 Photoshop；
-   所有文件格式均可人工编辑；
-   所有 Office 高级功能；
-   复杂动画和视觉效果；
-   VBA / Macro 高兼容性。

Dozer 的差异化重点不是：

> **Human 可以在这里完成所有手工编辑。**

而是：

> **Human 可以在这里查看几乎所有工作文件，并让 Agent
> 精确完成绝大多数实际修改。**

------------------------------------------------------------------------

## 26. 长期产品形态

传统软件：

``` text
Human
 ↓
UI Controls
 ↓
Document
```

Copilot 类产品：

``` text
Human
 ↓
Editor
 ↕
AI Assistant
```

Dozer 希望探索：

``` text
          Human
        ↙       ↘
    View/Review   Intent
       ↓           ↓
     Viewer ←→ Agent
       ↑           ↓
       └─ Mutation ┘
```

Human 的主要工作逐渐变成：

> **看、判断、指出、要求、验收。**

Agent 的主要工作变成：

> **读取、理解、定位、修改、验证、解释。**

Viewer 则成为双方共享的工作空间。

因此，Dozer Agent-native Editor 最终不是：

> **一个加入 AI 功能的文件编辑器。**

而应该是：

> **一个围绕 Human-Agent 协作重新设计的通用文件工作空间。**

------------------------------------------------------------------------

## 27. 核心验收原则

未来任何 Viewer、Editor 或文件格式接入 Dozer 时，应依次判断：

1.  **Can View** ------ Human 能否可靠查看？
2.  **Can Locate** ------ Human 与 Agent 能否共享精确位置？
3.  **Can Select** ------ Human 能否明确指出文件内部对象？
4.  **Can Contextualize** ------ Agent 能否可靠读取相关上下文？
5.  **Can Mutate** ------ Agent 能否执行精确修改？
6.  **Can Refresh** ------ 修改结果能否立即显示？
7.  **Can Relocate** ------ 修改后能否回到对应位置？
8.  **Can Review** ------ Human 能否清楚看到修改内容？
9.  **Can Trace** ------ 修改能否追溯到任务、来源和 Agent？
10. **Can Revert** ------ 修改能否安全撤销？

这十项可以作为未来 Dozer 文件组件的统一能力检查表。

------------------------------------------------------------------------

## 28. 一句话定义

> **Dozer Agent-native Editor 是一个以 Agent 为主要编辑执行者、以 Human
> 为阅读与决策主体，通过共享视野、精确定位、结构化修改和即时反馈形成
> Human-Agent 文件协作闭环的通用文件工作空间。**

核心产品公式：

> **Viewer + Shared Viewport + Typed Selection + Context Scope +
> Mutation Engine + Change Feedback**

核心工程原则：

> **Anchored + Scoped + Transactional + Traceable + Reversible**

最终目标：

> **Human 看得到，Agent 找得到；Human 指得准，Agent 改得准；Agent
> 改完，Human 立即看得到。**
