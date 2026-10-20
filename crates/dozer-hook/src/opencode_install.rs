//! OpenCode 插件安装：把 `dozer.ts`/`dozer-translate.ts` 写进
//! `~/.config/opencode/plugins/`（只放复数目录——spike
//! `2026-07-31-opencode-plugin-spike-findings.md` Step 2 实测确认单数
//! `plugin/` 目录会导致同一事件被同时加载两次触发两次，后续代码不碰
//! 它）。跟 Claude/CodeBuddy 的 JSON 增量合并不同，这两个文件是 Dozer
//! 独占的文件名，不与用户/其他工具共享同一份配置——install 直接整体
//! 覆盖写，uninstall 直接删，不需要合并逻辑。
//!
//! `dozer-translate.ts` 写进 `plugins/dozer-lib/` 子目录，而不是跟
//! `dozer.ts` 平铺在 `plugins/` 里——opencode 会把 `plugins/` 目录下每个
//! `*.ts`/`*.js` 文件当成独立插件加载，对文件里每个具名导出都当
//! `Plugin` 工厂调用一次。`dozer-translate.ts` 导出的是纯函数
//! （`onSessionCreated` 等），平铺时会被 opencode 拿插件初始化参数去调
//! 这些函数，触发 `undefined is not an object (evaluating
//! 'info.parentID')` 之类的运行时错误——2026-08-05 最终评审用真实
//! opencode 1.18.11 装机复现确认。塞进子目录能让 opencode 的平铺式自动
//! 发现完全看不到它，同时 `dozer.ts` 用相对路径 import 照常能找到它。
//!
//! `dozer.ts` 里 spawn `dozer-hook` 用的二进制路径在安装时被替换成
//! `current_exe()` 的绝对路径（跟 Claude/CodeBuddy 安装器把 exe 路径写
//! 进 hook command 字符串是同一手法），避免依赖插件运行时的 PATH。路径
//! 里的 `"`/`\` 会被转义，防止小概率把 TS 源码写坏。

use std::path::{Path, PathBuf};

const DOZER_TS_TEMPLATE: &str = include_str!("../opencode_plugin/dozer.ts");
const TRANSLATE_TS: &str = include_str!("../opencode_plugin/dozer-translate.ts");
const HOOK_BIN_PLACEHOLDER: &str = "__DOZER_HOOK_BIN_PATH__";
// 源码树里 `dozer.ts` 从平铺的 `./dozer-translate` 导入（跟
// `dozer-translate.test.ts` 同目录，`bun test` 能直接跑，也是这次改动的
// 点——之前这个导入路径写死指向 `./dozer-lib/dozer-translate`，只有装机
// 后才存在的子目录，源码树里跑不起来，没法直接测）。装机时才改写成子目录
// 路径，产出文件不变。
const TRANSLATE_IMPORT_SRC: &str = "\"./dozer-translate\"";

/// 插件目录路径，`DOZER_OPENCODE_PLUGIN_DIR` 覆盖用于测试（跟
/// `install.rs` 的 `DOZER_CLAUDE_SETTINGS`/`DOZER_CODEBUDDY_SETTINGS`
/// 同一套手法）。
pub fn plugins_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DOZER_OPENCODE_PLUGIN_DIR") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    PathBuf::from(home)
        .join(".config")
        .join("opencode")
        .join("plugins")
}

/// 转义 `\`/`"`：exe 路径若含这两个字符（极少见，取决于安装位置），直
/// 接拼进 TS 字符串字面量会产出语法错误的源码，导致插件静默加载失败。
fn escape_ts_string_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// CLI 场景（`dozer-hook install opencode`）用：调用方就是 `dozer-hook`
/// 自身，`current_exe()` 天然指向正确的二进制。GUI 场景（`dozer-app` 在
/// agent 启动时静默自动注册）不能走这条路——见 `run_at_with_exe`。
pub fn run_at(dir: &Path, install: bool) -> i32 {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "dozer-hook".into());
    run_at_with_exe(dir, install, &exe)
}

/// `run_at` 的可测试内核：exe 路径由调用方显式传入。`dozer-app` 在自己
/// 进程内直接调用这层——沿用 `current_exe()` 会把 `dozer.ts` 里 spawn 的
/// 二进制写成 `dozer-app` 自己的可执行文件路径，而不是 `dozer-hook` 的
/// （跟 `install::run_at_with_exe` 同一个根因，同一次线上事故）。
pub fn run_at_with_exe(dir: &Path, install: bool, exe: &str) -> i32 {
    run_at_with_exe_for(dir, install, exe, dozer_core::paths::product())
}

/// `run_at_with_exe` 的产品参数化内核:插件文件名(`dozer.ts`/`<name>.ts`)、
/// `<name>-lib` 子目录名与安装后的 translate import 路径全部按传入的
/// `product` 推导(复审 R1:import 路径曾写死 `./dozer-lib/...`,Digger
/// 的插件会去 import Dozer 的库目录——Dozer 一卸载,Digger 的插件随之
/// 失效)。测试用它构造另一个产品的并存场景,不依赖进程级 install。
pub fn run_at_with_exe_for(
    dir: &Path,
    install: bool,
    exe: &str,
    product: &dozer_core::paths::ProductId,
) -> i32 {
    let plugin_ts = format!("{}.ts", product.name);
    let lib_dir_name = format!("{}-lib", product.name);
    // 安装后的 import 必须指向本产品自己的库目录,与写盘的 translate
    // 文件位置逐字对应;源码树里的平铺 import(TRANSLATE_IMPORT_SRC)
    // 在装机时被整体替换成这个路径。
    let translate_import_installed = format!("\"./{lib_dir_name}/dozer-translate\"");
    if install {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("建目录失败: {e}");
            return 1;
        }
        let lib_dir = dir.join(&lib_dir_name);
        if let Err(e) = std::fs::create_dir_all(&lib_dir) {
            eprintln!("建目录失败: {e}");
            return 1;
        }
        let escaped_exe = escape_ts_string_literal(exe);
        let dozer_ts = DOZER_TS_TEMPLATE
            .replace(HOOK_BIN_PLACEHOLDER, &escaped_exe)
            .replace(TRANSLATE_IMPORT_SRC, &translate_import_installed);
        if let Err(e) = std::fs::write(dir.join(&plugin_ts), dozer_ts) {
            eprintln!("写 {plugin_ts} 失败: {e}");
            return 1;
        }
        if let Err(e) = std::fs::write(lib_dir.join("dozer-translate.ts"), TRANSLATE_TS) {
            eprintln!("写 dozer-translate.ts 失败: {e}");
            return 1;
        }
        println!("已安装: {}", dir.display());
    } else {
        let translate_ts = dir.join(&lib_dir_name).join("dozer-translate.ts");
        if translate_ts.exists()
            && let Err(e) = std::fs::remove_file(&translate_ts)
        {
            eprintln!("删 {} 失败: {e}", translate_ts.display());
            return 1;
        }
        // 尽力而为：子目录若已空就顺手删掉，删不掉（不存在/非空/权限）
        // 不算卸载失败。
        let _ = std::fs::remove_dir(dir.join(&lib_dir_name));
        let dozer_ts = dir.join(&plugin_ts);
        if dozer_ts.exists()
            && let Err(e) = std::fs::remove_file(&dozer_ts)
        {
            eprintln!("删 {} 失败: {e}", dozer_ts.display());
            return 1;
        }
        println!("已卸载: {}", dir.display());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dozer 身份下安装产物里的 translate import 路径(复审 R1 前,这个
    /// 值曾写死在生产代码常量里;现在由 `run_at_with_exe_for` 按产品
    /// 动态生成,期望字符串保留在测试侧钉住 Dozer 身份的产物不变)。
    const TRANSLATE_IMPORT_INSTALLED: &str = "\"./dozer-lib/dozer-translate\"";

    /// Digger 身份夹具,与 `install.rs` 的并存测试同一套手法。
    fn digger_product(root: &std::path::Path) -> dozer_core::paths::ProductId {
        use dozer_core::paths::{ProductDirs, ProductId};
        ProductId::custom(
            "digger",
            "DIGGER_SESSION_ID",
            "digger-hook",
            "diggerd.sock",
            "digger.db",
            ProductDirs::Fixed {
                config: root.join("config"),
                state: root.join("state"),
            },
        )
    }

    /// 复审 R1:Digger 身份安装时,插件文件是 `digger.ts`,translate import
    /// 指向 Digger 自己的 `digger-lib`,目标文件真实存在,文本里不出现
    /// Dozer 的 `./dozer-lib`。
    #[test]
    fn digger_install_writes_product_named_plugin_with_matching_import() {
        let dir = tempfile::tempdir().unwrap();
        let digger = digger_product(dir.path());
        assert_eq!(
            run_at_with_exe_for(dir.path(), true, "/opt/digger-hook", &digger),
            0
        );
        let plugin = std::fs::read_to_string(dir.path().join("digger.ts")).unwrap();
        assert!(
            dir.path().join("digger-lib/dozer-translate.ts").exists(),
            "translate 必须落在 digger-lib/ 里"
        );
        assert!(
            plugin.contains("\"./digger-lib/dozer-translate\""),
            "import 必须指向 digger-lib"
        );
        assert_eq!(
            plugin.matches("\"./digger-lib/dozer-translate\"").count(),
            1
        );
        assert!(!plugin.contains("./dozer-lib"), "不得再引用 Dozer 的库目录");
        assert!(
            !dir.path().join("dozer.ts").exists() && !dir.path().join("dozer-lib").exists(),
            "Digger 安装不得写 Dozer 名下的文件"
        );
    }

    /// 复审 R1:同一插件目录里两个产品并存,互不引用;卸载任一产品,
    /// 另一个的插件与库目录完整无损。
    #[test]
    fn digger_and_dozer_opencode_plugins_coexist_and_uninstall_leaves_the_other_intact() {
        let dir = tempfile::tempdir().unwrap();
        let digger = digger_product(dir.path());
        let dozer = dozer_core::paths::ProductId::dozer();
        assert_eq!(
            run_at_with_exe_for(dir.path(), true, "/opt/dozer-hook", &dozer),
            0
        );
        assert_eq!(
            run_at_with_exe_for(dir.path(), true, "/opt/digger-hook", &digger),
            0
        );
        // 两套文件并存
        for f in [
            "dozer.ts",
            "dozer-lib/dozer-translate.ts",
            "digger.ts",
            "digger-lib/dozer-translate.ts",
        ] {
            assert!(dir.path().join(f).exists(), "缺 {f}");
        }
        let dozer_ts = std::fs::read_to_string(dir.path().join("dozer.ts")).unwrap();
        let digger_ts = std::fs::read_to_string(dir.path().join("digger.ts")).unwrap();
        assert!(dozer_ts.contains("\"./dozer-lib/dozer-translate\""));
        assert!(!dozer_ts.contains("digger-lib"));
        assert!(digger_ts.contains("\"./digger-lib/dozer-translate\""));
        assert!(!digger_ts.contains("./dozer-lib"));
        // 卸载 Digger:Dozer 一侧原样
        assert_eq!(
            run_at_with_exe_for(dir.path(), false, "/opt/digger-hook", &digger),
            0
        );
        assert!(!dir.path().join("digger.ts").exists());
        assert!(!dir.path().join("digger-lib").exists());
        assert!(dir.path().join("dozer.ts").exists());
        assert!(dir.path().join("dozer-lib/dozer-translate.ts").exists());
        let dozer_ts_after = std::fs::read_to_string(dir.path().join("dozer.ts")).unwrap();
        assert_eq!(
            dozer_ts, dozer_ts_after,
            "卸载 Digger 不得改动 Dozer 的插件"
        );
    }

    #[test]
    fn install_writes_both_files_with_exe_path_substituted() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        let dozer_ts = std::fs::read_to_string(dir.path().join("dozer.ts")).unwrap();
        assert!(
            !dozer_ts.contains("__DOZER_HOOK_BIN_PATH__"),
            "占位符必须被替换成真实 exe 路径"
        );
        assert!(dozer_ts.contains("DozerPlugin"));
        let translate_ts =
            std::fs::read_to_string(dir.path().join("dozer-lib").join("dozer-translate.ts"))
                .unwrap();
        assert!(translate_ts.contains("onSessionCreated"));
    }

    #[test]
    fn run_at_with_exe_uses_the_passed_exe_not_current_exe() {
        // ensure_hook_installed（dozer-app）调这层是为了绕开 current_exe()
        // 在跨进程场景下拿错二进制的问题——这里直接断言传参优先。
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at_with_exe(dir.path(), true, "/opt/dozer-hook"), 0);
        let dozer_ts = std::fs::read_to_string(dir.path().join("dozer.ts")).unwrap();
        assert!(dozer_ts.contains("/opt/dozer-hook"), "{dozer_ts}");
    }

    #[test]
    fn install_puts_translate_ts_in_lib_subdir_not_flat() {
        // Critical: opencode 平铺加载 plugins/ 目录下每个 *.ts 文件当插
        // 件，把 dozer-translate.ts 的具名导出（纯函数）当 Plugin 工厂调
        // 用会直接报错。必须塞进子目录，让平铺式自动发现看不到它。
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        assert!(
            !dir.path().join("dozer-translate.ts").exists(),
            "dozer-translate.ts 不能直接平铺在 plugins/ 下，否则会被 opencode 当插件加载"
        );
        assert!(
            dir.path()
                .join("dozer-lib")
                .join("dozer-translate.ts")
                .exists()
        );
        let dozer_ts = std::fs::read_to_string(dir.path().join("dozer.ts")).unwrap();
        assert!(
            dozer_ts.contains("./dozer-lib/dozer-translate"),
            "dozer.ts 的 import 路径必须指向子目录"
        );
    }

    #[test]
    fn install_rewrites_source_tree_import_path_not_leaves_both() {
        // 源码树里 `dozer.ts` 平铺 import `./dozer-translate`（这样
        // `bun test` 能在源码树直接跑，不必装机）；装机产物必须整体替换成
        // 子目录路径，不能两个 import 语句都留着（重复导入/语法错误）。
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        let dozer_ts = std::fs::read_to_string(dir.path().join("dozer.ts")).unwrap();
        assert_eq!(
            dozer_ts.matches(TRANSLATE_IMPORT_SRC).count(),
            0,
            "平铺的源码树 import 路径必须被整体替换掉，不能留在装机产物里"
        );
        assert_eq!(
            dozer_ts.matches(TRANSLATE_IMPORT_INSTALLED).count(),
            1,
            "import 语句只能出现一次，装机后必须是子目录那个版本"
        );
    }

    #[test]
    fn escape_ts_string_literal_handles_quotes_and_backslashes() {
        // Minor finding 5：exe 路径含 `"`/`\` 时必须被转义，否则拼进 TS
        // 字符串字面量会产出语法错误的源码。
        let raw = r#"/opt/weird "path"\dozer-hook"#;
        let escaped = escape_ts_string_literal(raw);
        assert_eq!(escaped, r#"/opt/weird \"path\"\\dozer-hook"#);
        // 拼进真实模板后必须是语法合法的双引号字符串字面量内容——不能
        // 出现未转义的 `"` 提前结束字符串。
        let ts = format!("const x = \"{escaped}\"");
        assert_eq!(ts.matches('"').count(), 4); // 开/闭各一对，内部两个都转义过
    }

    #[test]
    fn install_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        assert_eq!(run_at(dir.path(), true), 0);
        assert!(dir.path().join("dozer.ts").exists());
    }

    #[test]
    fn uninstall_removes_both_files() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_at(dir.path(), true), 0);
        assert_eq!(run_at(dir.path(), false), 0);
        assert!(!dir.path().join("dozer.ts").exists());
        assert!(
            !dir.path()
                .join("dozer-lib")
                .join("dozer-translate.ts")
                .exists()
        );
        // 空的 dozer-lib/ 子目录也该被顺手清掉。
        assert!(!dir.path().join("dozer-lib").exists());
    }

    #[test]
    fn uninstall_on_missing_dir_does_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("not-created-yet");
        assert_eq!(run_at(&nested, false), 0);
    }

    #[test]
    fn plugins_dir_honors_env_override() {
        unsafe { std::env::set_var("DOZER_OPENCODE_PLUGIN_DIR", "/tmp/probe-opencode-plugins") };
        assert_eq!(plugins_dir(), PathBuf::from("/tmp/probe-opencode-plugins"));
        unsafe { std::env::remove_var("DOZER_OPENCODE_PLUGIN_DIR") };
    }
}
