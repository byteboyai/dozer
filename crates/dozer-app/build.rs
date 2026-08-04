//! 把 `packaging/macos/Info.plist` 嵌进 Mach-O 二进制(`__TEXT,__info_plist`
//! 段),让 `cargo run` 直接跑的裸二进制也认得里面的 `NSAppTransportSecurity`
//! 例外——否则 macOS 默认 ATS 会拦掉浏览器子视图(WKWebView)加载的不加密
//! `http://`/`http://localhost` 网页,表现就是"输入网址后页面空白"。
//! 只有打 .app 包时 Info.plist 才天然生效;开发期 `cargo run` 是裸二进制、
//! 没有 bundle,必须靠内嵌段补上。`cargo build` 链接时由 rustc 把这条
//! `-Wl,-sectcreate` 透传给 ld 完成嵌入。
//!
//! 仅 macOS 需要;Windows/Linux 无 ATS,直接 no-op。
fn main() {
    #[cfg(target_os = "macos")]
    {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 未设置");
        let plist = std::path::Path::new(&manifest_dir).join("packaging/macos/Info.plist");
        if plist.exists() {
            println!(
                "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
                plist.display()
            );
        }
    }
}
