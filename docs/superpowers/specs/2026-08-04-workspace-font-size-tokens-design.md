# workspace.rs 字号 token 化(fonts.json)

> 状态:设计中,待用户终审。
> 需求来源:2026-08-04 对话(用户先要求把 `regions.json` 的 `background` 从令牌名改成具体色值方便
> 自定义主题,随后追加"把字体配置也放入 json 中")。
> 上游背景:`docs/superpowers/specs/2026-07-30-dozer-shell-chrome-style-config-design.md`(`regions.json`
> + `chrome_style.rs` 的既有模式,本设计直接复用其加载/解析套路)。同一轮对话里 `chrome_style.rs`
> 的 `resolve_color` 已扩展为同时支持令牌名和 `#RRGGBB[AA]` 十六进制字面量。

## 1. 目标与动机

`workspace.rs` 里 `text(...).size(N)` 的字号全是散落字面量,统计下来 87 处只用了 8 个不同数值
(8/9/10/11/12/13/14/15),但没有名字、没有集中定义、改一个字号要满仓库搜数字。参照 `regions.json`
已经把颜色/边框/间距配置化的先例,把这 8 个数值收敛成一套具名 token,写进编译期内嵌的 JSON,让
字号也变成"改一处、查得到、有名字",顺带打开用户后续自定义字号的口子。

**非目标(本轮明确排除)**:
- 不改变任何现有视觉效果——纯粹的字面量搬家,数值本身不变,不趁机拉齐/合并相近字号。
- 不做运行时读盘/热更新——JSON 编译期内嵌进二进制,和 `regions.json`/`theme.rs` 现在的定位一致。
- 不涉及字体家族(family)、字重(weight)、行高——本轮只解决"字号"这一个维度。终端渲染
  (`term_view.rs` 的 `FONT_SIZE`/`CELL_WIDTH`/行高系数,等宽字体)不在范围内,那是独立的终端网格
  定位机制,和 `workspace.rs` 的普通文本字号不是同一套东西。
- 不覆盖 `workspace.rs` 之外的文件——已确认全仓库只有 `workspace.rs` 用到这些数字字面量字号。

## 2. 现状盘点:8 个数值,87 处调用

```
.size(13) × 34   .size(11) × 16   .size(14) × 15   .size(15) × 9
.size(12) × 7    .size(10) × 2    .size(9)  × 3    .size(8)  × 1
```

全部是整数字面量(无变量、无 `Pixels(...)` 包装),8/9/10 三档几乎都是状态圆点 `"●"` 或超小标签,
11-15 是正文/标题类文字。8 个数值各建一个 token,不合并档位、不新增档位,1:1 迁移:

| token | 数值 | 典型用途(参考现有调用点) |
|---|---|---|
| `dot_xs` | 8 | 文件树状态圆点 |
| `dot_sm` | 9 | 卡片/工具栏状态圆点 |
| `caption_sm` | 10 | 极小标签(如卡片副标题第二行) |
| `caption` | 11 | 小号说明文字(路径、次要元信息) |
| `label` | 12 | 次级正文/标签(思考折叠行、错误提示) |
| `body` | 13 | 默认正文(出现频次最高,34/87) |
| `subtitle` | 14 | 强调正文/小标题(对话气泡正文、按钮大字) |
| `title` | 15 | 标题类文字(顶栏 "Dozer"、卡片标题、文件树主文字) |

## 3. Schema:平铺 token 表

```json
{
  "dot_xs": 8,
  "dot_sm": 9,
  "caption_sm": 10,
  "caption": 11,
  "label": 12,
  "body": 13,
  "subtitle": 14,
  "title": 15
}
```

没有 `regions.json` 那种嵌套结构(背景/边框/padding/gap 组合)的必要——这里只有一个维度(数值),
直接平铺 `token → 数值` 即可。

## 4. 加载与消费机制

- 新文件:`crates/dozer-app/assets/theme/fonts.json`。
- 新模块:`crates/dozer-app/src/font_style.rs`,严格镜像 `chrome_style.rs` 的既有模式:
  - `include_str!("../assets/theme/fonts.json")` 编译期内嵌。
  - `std::sync::LazyLock<FontSizes>`,程序生命周期内解析一次:
    `serde_json::from_str(RAW).expect("fonts.json 格式错误")`——解析失败是开发期配置错误,直接
    panic,不做运行时降级。
  - 8 个访问函数,如 `pub fn body() -> u32`、`pub fn title() -> u32`——`iced_core::Pixels` 只对
    `f32`/`u32` 实现 `From`,现有 `.size(13)` 这类无后缀整数字面量正是靠 `u32` 这条 impl 才能编译
    通过,故返回类型定为 `u32`,调用点改动量最小:`.size(13)` → `.size(font_style::body())`。
- **不**与 `chrome_style.rs`/`regions.json` 合并到同一份文件或同一个模块——`chrome_style.rs` 头部
  注释明确写了"只覆盖区域外层容器样式,区域内部控件样式不在这里",字号属于控件内部文字,和
  `regions.json` 的既有职责边界不同,合并会打破这条已写明的边界。

## 5. 调用侧改动

`workspace.rs` 里全部 87 处 `.size(N)` 按 §2 的映射表逐一替换成 `.size(font_style::xxx())`,一次性
全量迁移(非部分骨架、非渐进式)。替换靠"当前数值 → 对应 token"精确匹配,替换后数值不变,视觉
应像素级一致。

## 6. 测试与验收方式

- **回归测试(防漂移锚)**:仿 `chrome_style.rs` 已有套路——为 `fonts.json` 里每个 token 写一条
  断言其解析结果等于迁移前字面量数值的测试(如 `assert_eq!(font_style::body(), 13u32)`)。
- **异常路径测试**:JSON 缺字段/字段类型错误触发 panic 的测试(镜像 `chrome_style.rs` 现有的
  `unknown_color_token_panics` 套路)。
- **视觉验收**:编译通过 + 真机跑起来目测,确认改动前后像素级一致(纯代码搬家,不应有任何肉眼
  可见变化)。

## 7. 影响范围小结

- 新增:`crates/dozer-app/assets/theme/fonts.json`、`crates/dozer-app/src/font_style.rs`。
- 修改:`crates/dozer-app/src/workspace.rs`(87 处 `.size(N)` 改为读 `font_style::`)、
  `crates/dozer-app/src/main.rs`(或对应的 `mod` 声明入口,新增 `mod font_style;`)。
- 不涉及 `term_view.rs`(终端字号是独立机制,见 §1 非目标)、`dozerd`/`dozer-core`/`dozer-hook`/
  `legacy-boy`,纯 `dozer-app` GUI 内部改动。
