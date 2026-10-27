//! 产品注入 / 产出的适配类型。

/// 浏览器页签期望的 webview(纯数据):由库产出,宿主转成自己的 webview 池规格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserWebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
}
