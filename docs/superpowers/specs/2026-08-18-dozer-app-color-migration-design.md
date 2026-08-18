# dozer-app 迁移 theme::color 到 byteui

**状态:已批准(brainstorming 会话,2026-08-18)**

## 背景

这是 `byteui` 落地后 4 个迁移子项目里的第 2 个(顺序:`interaction` →
**`theme::color`** → `theme::font`+`theme::geometry` → `theme::icon_size`)。
`byteui::theme::color`(见 `2026-08-18-byteui-library-design.md`)已经把
ByteBoy2077 的 23 个颜色 token 做成运行时可替换的 `ColorTokens` 单例
(`current()`/`set_theme()`),`dozer-app` 的 `theme/color.rs` 现在还是一份
逐字节独立、值却本该一致的重复实现——不删掉它,以后改一个色值就有两处要
同步改、忘改一处就是视觉不一致的 bug。

**前提:本迁移假设子项目 1(`interaction`)已经合并到 main。** 迁移完成后
`crates/dozer-app/src/{icons.rs,scrollbar.rs}` 已不存在,`Cargo.toml` 已有
`byteui` 依赖——这次不需要重新加依赖,也不需要处理这两个文件。

摸底时发现一处 migration #1 设计阶段没预料到的情况:`crates/dozer-app/src/
theme/region.rs` 用 `use super::color;` 把 `color` 这个模块别名引进作用域,
之后全用裸 `color::BG` 这种短路径调用(不是 `theme::color::BG` 完整路径)。
这是唯一一个用这种写法的文件——其余全部走完整限定路径,替换规则要按两种
不同形状分别处理,不能用同一套 sed 规则套所有文件。

**迁移策略延续 migration #1 已定的"全量改调用点"**:不留任何本地重导出层,
每个调用点直接指向 `byteui::theme::color::...`。

## 目标 / 非目标

**目标**:

1. 19 个文件(`theme::color::NAME` 完整路径形式,632 处,含 4 处
   `theme::color::mix(...)` 函数调用)+ `region.rs`(裸 `color::NAME` 形式,
   30 处)共 20 个文件、662 处调用点,把大写常量引用改成
   `byteui::theme::color::current().<小写字段名>`,函数调用 `mix(...)`
   改成 `byteui::theme::color::mix(...)`。
2. 删除 `crates/dozer-app/src/theme/color.rs`,删除 `theme/mod.rs` 里的
   `pub mod color;`。
3. `region.rs` 额外删掉 `use super::color;` 这一行(不再需要,替换后全部
   引用都已完整限定,不依赖这个别名)。
4. 迁移完成后 `dozer-app` 编译、测试、clippy、fmt 全绿,GUI 视觉与迁移前
   逐一比对无差异——颜色是全 App 使用面最广的 token,这次视觉核对要覆盖
   的范围比 migration #1(只有 icon/tab/card/scrollbar)大得多。

**非目标**:

- **不迁移 `theme::font`/`theme::geometry`/`theme::icon_size`**——后续两个
  独立子项目的范围。
- **不改变任何视觉效果**。`byteui::theme::color::current().gold` 和被删除
  的 `crate::theme::color::GOLD` 取值逐字节一致(`ColorTokens::
  byteboy2077()` 的 23 个字段值和 `dozer-app` 原 `theme/color.rs` 的 23 个
  常量一一对应,已在 `byteui` 建库时核对锁死)。
- **不改动 `theme/homespace_color.rs`/`theme/homespace_font.rs`**——这是
  首页(homespace)独立的一套配色系统,和 ByteBoy2077 主题
  (`theme::color`)无关联,不在这次范围内。

## 架构与数据流

### 前置检查(执行前先确认)

```bash
grep -c "byteui" crates/dozer-app/Cargo.toml   # 应 ≥ 1,确认 migration #1 已加依赖
ls crates/dozer-app/src/icons.rs 2>&1          # 应报 No such file,确认 migration #1 已删本地文件
```

如果这两条确认失败,说明 migration #1 还没合并,先去完成/合并那个,不要
在它之前开始这次迁移。

### 常量名 → `ColorTokens` 字段名对照表(23 个,逐一对应,不得增减改名)

| 大写常量 | 小写字段 | 大写常量 | 小写字段 |
|---|---|---|---|
| `BG` | `bg` | `IGNORED` | `ignored` |
| `PANEL` | `panel` | `ORANGE` | `orange` |
| `TERM_BG` | `term_bg` | `MAGENTA` | `magenta` |
| `CARD` | `card` | `BLUE` | `blue` |
| `BORDER` | `border` | `LIME` | `lime` |
| `CREAM` | `cream` | `SCRIM` | `scrim` |
| `BODY` | `body` | `TAB_ACTIVE_BORDER` | `tab_active_border` |
| `DIM` | `dim` | `TAB_ACTIVE_BG` | `tab_active_bg` |
| `GOLD` | `gold` | `TAB_HOVER` | `tab_hover` |
| `CYAN` | `cyan` | `DESC_BG` | `desc_bg` |
| `GREEN` | `green` | | |
| `PURPLE` | `purple` | | |
| `RED` | `red` | | |

macOS 自带的 BSD `sed` 不支持替换文本里的大小写转换(`\L`/`\U` 是 GNU sed
扩展),所以下面两组替换规则要把 23 条逐一写全,不能靠正则自动转小写。

**`\b` 词边界在 macOS 的 BSD sed 上不生效**(已实测确认:`sed -E
's/GOLD\b/X/'` 在 BSD sed 上不匹配任何东西,整条规则静默失效,不报错——
比留着更危险)。规则组里一律不用 `\b`,改成靠 `::` 分隔符本身当边界:已经
逐一核对过,23 个常量名没有一个是另一个的前缀(比如 `TAB_ACTIVE_BG` 和
`TAB_ACTIVE_BORDER` 共同前缀到 `TAB_ACTIVE_B` 就分叉,谁都不是谁的前缀),
所以 `theme::color::BG` 这样的模式只会匹配"`color::` 后面恰好紧跟 `BG`
接非字母数字字符(或行尾)"的位置,不会误伤 `theme::color::TAB_ACTIVE_BG`
(那里 `color::` 后面紧跟的是 `TAB_ACTIVE_BG` 不是 `BG`)。

### 替换规则组 A(19 个文件,`theme::color::NAME` 完整路径形式)

**先归一化 `crate::` 前缀,再套 23 条规则。** 已核对:`extensions/
database.rs`(不含 `icons.rs`,那个已被 migration #1 删除)和 `extensions/
ssh/sftp.rs` 里有 50 处写的是 `crate::theme::color::NAME`(带显式
`crate::` 前缀)而不是靠 `use crate::theme;` 引入的裸 `theme::color::NAME`
——如果直接套下面 23 条规则,`crate::theme::color::GOLD` 会变成
`crate::byteui::theme::color::current().gold`,而 `byteui` 是外部 crate
不是 `crate`(即 `dozer-app` 自己)下的子模块,这样写编译不过。所以第一条
规则必须先把 `crate::theme::color::` 归一成 `theme::color::`,再让后面
的规则统一处理:

```bash
sed -i '' \
  -e 's/crate::theme::color::/theme::color::/g' \
  -e 's/theme::color::mix(/byteui::theme::color::mix(/g' \
  -e 's/theme::color::BG/byteui::theme::color::current().bg/g' \
  -e 's/theme::color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/theme::color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/theme::color::CARD/byteui::theme::color::current().card/g' \
  -e 's/theme::color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/theme::color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/theme::color::BODY/byteui::theme::color::current().body/g' \
  -e 's/theme::color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/theme::color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/theme::color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/theme::color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/theme::color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/theme::color::RED/byteui::theme::color::current().red/g' \
  -e 's/theme::color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/theme::color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/theme::color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/theme::color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/theme::color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/theme::color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/theme::color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/theme::color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/theme::color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/theme::color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  <file>
```

`mix(` 规则放第一条,原因:它匹配的是小写函数名,和后面 23 条全大写常量
规则的匹配范围不重叠,顺序不影响结果,放前面只是习惯。

### 替换规则组 B(仅 `region.rs`,裸 `color::NAME` 形式)

和规则组 A 结构一致,只是每条模式里去掉 `theme::` 前缀(`color::BG` 而非
`theme::color::BG`):

```bash
sed -i '' \
  -e 's/color::BG/byteui::theme::color::current().bg/g' \
  -e 's/color::PANEL/byteui::theme::color::current().panel/g' \
  -e 's/color::TERM_BG/byteui::theme::color::current().term_bg/g' \
  -e 's/color::CARD/byteui::theme::color::current().card/g' \
  -e 's/color::BORDER/byteui::theme::color::current().border/g' \
  -e 's/color::CREAM/byteui::theme::color::current().cream/g' \
  -e 's/color::BODY/byteui::theme::color::current().body/g' \
  -e 's/color::DIM/byteui::theme::color::current().dim/g' \
  -e 's/color::GOLD/byteui::theme::color::current().gold/g' \
  -e 's/color::CYAN/byteui::theme::color::current().cyan/g' \
  -e 's/color::GREEN/byteui::theme::color::current().green/g' \
  -e 's/color::PURPLE/byteui::theme::color::current().purple/g' \
  -e 's/color::RED/byteui::theme::color::current().red/g' \
  -e 's/color::IGNORED/byteui::theme::color::current().ignored/g' \
  -e 's/color::ORANGE/byteui::theme::color::current().orange/g' \
  -e 's/color::MAGENTA/byteui::theme::color::current().magenta/g' \
  -e 's/color::BLUE/byteui::theme::color::current().blue/g' \
  -e 's/color::LIME/byteui::theme::color::current().lime/g' \
  -e 's/color::SCRIM/byteui::theme::color::current().scrim/g' \
  -e 's/color::TAB_ACTIVE_BORDER/byteui::theme::color::current().tab_active_border/g' \
  -e 's/color::TAB_ACTIVE_BG/byteui::theme::color::current().tab_active_bg/g' \
  -e 's/color::TAB_HOVER/byteui::theme::color::current().tab_hover/g' \
  -e 's/color::DESC_BG/byteui::theme::color::current().desc_bg/g' \
  crates/dozer-app/src/theme/region.rs
```

已核对 `region.rs` 里实际出现的 14 个常量名(`BG`/`BODY`/`BORDER`/`CARD`/
`CREAM`/`CYAN`/`DIM`/`GOLD`/`GREEN`/`PANEL`/`PURPLE`/`RED`/`SCRIM`/
`TERM_BG`)全部落在这 23 条规则内,规则组 B 覆盖完整,没有遗漏的常量名;
`region.rs` 没有 `color::mix(` 调用(已核对),规则组 B 不需要 mix 那条。

**规则组 B 只能用在 `region.rs` 一个文件上**——如果误用在规则组 A 的文件
上,`color::GOLD` 会作为 `theme::color::GOLD` 的子串被二次误伤,产出错误
的双重替换(`byteui::theme::byteui::theme::color::current().gold` 这种)。

`region.rs` 顶部的 `use super::color;`(第 21 行)在替换后不再被任何代码
引用,需要单独删掉这一行,否则编译期会报 unused import 警告。

### 受影响文件与调用点数

**规则组 A(19 个文件,632 处)**:

| 文件 | 调用点数 |
|---|---|
| `crates/dozer-app/src/workspace.rs` | 100 |
| `crates/dozer-app/src/extensions/todo.rs` | 91 |
| `crates/dozer-app/src/app.rs` | 65 |
| `crates/dozer-app/src/extensions/files.rs` | 55 |
| `crates/dozer-app/src/extensions/ssh.rs` | 48 |
| `crates/dozer-app/src/extensions/project.rs` | 47 |
| `crates/dozer-app/src/extensions/git_log.rs` | 40 |
| `crates/dozer-app/src/extensions/database.rs` | 36 |
| `crates/dozer-app/src/extensions/acceptance.rs` | 26 |
| `crates/dozer-app/src/extensions/usage.rs` | 24 |
| `crates/dozer-app/src/preview.rs` | 23 |
| `crates/dozer-app/src/extensions/search.rs` | 23 |
| `crates/dozer-app/src/extensions/browser.rs` | 19 |
| `crates/dozer-app/src/extensions/footbar.rs` | 10 |
| `crates/dozer-app/src/term_view.rs` | 7 |
| `crates/dozer-app/src/extensions/ssh/sftp.rs` | 7 |
| `crates/dozer-app/src/homespace.rs` | 3 |
| `crates/dozer-app/src/menu.rs` | 4 |
| `crates/dozer-app/src/diff_render.rs` | 4 |

**规则组 B(1 个文件,30 处)**:

| 文件 | 调用点数 |
|---|---|
| `crates/dozer-app/src/theme/region.rs` | 30 |

## 错误处理

不适用——纯路径/语法重写(const 访问 → 函数调用取字段),没有新增可能
失败的运行时逻辑。编译器是主要校验手段:漏改或替换出错会在
`cargo build -p dozer-app` 时直接报错,不会静默产生错误行为。

## 测试策略

1. 每个文件改完后单独跑 `cargo build -p dozer-app --bin dozer`。
2. 全部 20 个文件改完后:`cargo test -p dozer-app --bin dozer`(基线同
   migration #1:484 passed / 2 failed,两个已知的、与本次改动无关的
   terminal grid 尺寸测试失败)、`cargo clippy -p dozer-app --all-targets
   -- -D warnings`、`cargo fmt -p dozer-app -- --check`。
3. 独立命名的临时二进制做一次**全面**视觉核对(比 migration #1 范围更大,
   因为颜色 token 覆盖全 App):
   - 四栏骨架背景色、边框色。
   - 各面板文字颜色(标题/正文/次要文字/dim 态)。
   - 状态色(成功绿/失败红/进行中青/警示橙等,尤其 `extensions/acceptance.rs`/
     `extensions/todo.rs`/`extensions/usage.rs` 这几个用色最密集的面板)。
   - 顶栏选中页签描边/背景(`TAB_ACTIVE_BORDER`/`TAB_ACTIVE_BG`)。
   - `region.rs` 驱动的各区域背景色(顶栏/左右 icon rail/各 pane)——这是
     规则组 B 唯一覆盖的文件,单独确认一遍。
   完成后关闭该临时实例、删除临时二进制,不留后台进程。

## 排期备注

20 个文件相互独立,可以按任意顺序做、分批提交分批审阅。删
`theme/color.rs` 和清理 `theme/mod.rs` 里的 `pub mod color;` 必须在 20 个
文件全部改完之后才能做(中间状态下 `theme/color.rs` 还要留着,否则未迁移
的文件编译不过)。
