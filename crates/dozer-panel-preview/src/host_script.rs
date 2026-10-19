//! 预览 webview 的常驻注入脚本(S5b-2 P3)。
//!
//! 从 `dozer-app::runtime` 原样搬来「不碰 wry 句柄」的那部分:焦点 / 拖拽结束 /
//! 缩放、文本光标与复制、剪切/粘贴/全选的页面内实现,以及 Flyfish host 的
//! ⌘F 交接。产品(Dozen 的 wry 池)只负责把它传给 `with_initialization_script`。
//!
//! 脚本字节是行为守恒的一部分:搬库前后必须逐字相同,由
//! `tests/fixtures/preview_host_script_golden.txt` 钉住(搬迁前在现状上生成一次)。
//!
//! 产品自己的段落(浏览器面板的标题回报 `title:<id>:<title>`、应用面板的语言桥)
//! 不在这里——预览面板不掺和。

/// 哪些常驻脚本段落要装上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostScriptOptions {
    /// editor host(`dozer://editor/`)是否要装文本光标/复制/剪切粘贴段落。
    ///
    /// Dozer 侧对应 `editor_binding.is_none()`:editor host 自带 CodeMirror 的
    /// 编辑行为,不再叠加页面级覆写;其它所有 webview(含浏览器面板)都装。
    pub text_editing: bool,
    /// 该 host 是 Flyfish(`dozer://flyfish/`):⌘F 交还给产品 Find 条;
    /// 否则 ⌘F 先找页面自带搜索框,找不到才退回原生查找。
    pub flyfish: bool,
}

/// 生成一个预览 webview 的初始化脚本(焦点/缩放 + 文本编辑 + ⌘F)。
pub fn preview_host_script(options: HostScriptOptions) -> String {
    let mut script = String::from(
        "document.addEventListener('mousedown',function(){window.ipc.postMessage('focus')},true);document.addEventListener('mouseup',function(){window.ipc.postMessage('mouseup')},true);document.addEventListener('keydown',function(e){if(e.ctrlKey){var c=e.code,k=e.key;if(c==='Equal'||k==='+'||k==='='){e.preventDefault();window.ipc.postMessage('zoom_in');}else if(c==='Minus'||k==='-'){e.preventDefault();window.ipc.postMessage('zoom_out');}else if(c==='Digit1'||k==='1'){e.preventDefault();window.ipc.postMessage('zoom_reset');}}},true);",
    );
    // 文本光标 + Ctrl/Cmd+C 复制(所有 webview):
    // - 默认让普通文字显示 I 形文本光标;链接/按钮等可交互元素
    //   仍是手形(继承自 `cursor: text` 时会被 `cursor: pointer`
    //   覆盖,因为后写、同为 `!important`)。
    // - Ctrl+C / Cmd+C 都拦截复制当前选区:webview 成为 first
    //   responder 后 keydown 被它吃掉、到不了 winit,得在这段
    //   注入 JS 里自己拦。用 `document.execCommand('copy')`
    //   (用户手势触发,WKWebView 会放行);个别方案拿不到选区
    //   或失败时再退回 `navigator.clipboard`。`Cmd+Meta` 双键
    //   都按,保证两套快捷键一致触发。用 `e.code` 判断,避免
    //   键盘布局差异影响 `e.key`。
    if options.text_editing {
        script.push_str(
            "(function(){var s=document.createElement('style');s.textContent=\"html,body,body *{cursor:text!important}a,a *,button,*[role='link'],*[role='button'],summary,*[onclick],label[for]{cursor:pointer!important}\";(document.head||document.documentElement).appendChild(s);document.addEventListener('keydown',function(e){if((e.ctrlKey||e.metaKey)&&e.code==='KeyC'){e.preventDefault();var ok=false;try{ok=document.execCommand('copy')}catch(_){}if(!ok){var g=window.getSelection&&window.getSelection();if(g&&g.toString()){try{navigator.clipboard.writeText(g.toString()).then(function(){},function(){})}catch(_){}}}}});})();",
        );
        // 剪切/粘贴/全选(所有 webview 的可编辑元素):本应用没有
        // macOS Edit 菜单,WKWebView 不派发 cut:/paste:/selectAll:,
        // 这三个组合键在页面输入框里默认全部失效。⌘X/⌘A 在页面内
        // 自己解决;⌘V 读剪贴板必须宿主代劳(WKWebView 禁止页面读
        // 剪贴板),经 IPC 发 `paste_request`、宿主回灌
        // `window.__dozerPaste(text)` 插进聚焦元素。
        script.push_str(
            "(function(){function _dozEdit(el){if(!el)return false;var t=el.tagName;if(t==='TEXTAREA'||el.isContentEditable)return true;if(t==='INPUT'){var y=el.type;return !y||y==='text'||y==='search'||y==='url'||y==='tel'||y==='password'||y==='number';}return false;}window.__dozerPaste=function(text){var el=document.activeElement;if(!_dozEdit(el))return;if(el.setRangeText){var s=el.selectionStart??0;el.setRangeText(text,s,el.selectionEnd??s,'end');el.setSelectionRange(s+text.length,s+text.length);el.dispatchEvent(new Event('input',{bubbles:true}));}else{try{document.execCommand('insertText',false,text)}catch(_){}}};document.addEventListener('keydown',function(e){if(!(e.ctrlKey||e.metaKey))return;var c=e.code;if(c==='KeyV'){if(_dozEdit(document.activeElement)){e.preventDefault();window.ipc&&window.ipc.postMessage('paste_request');}}else if(c==='KeyX'){e.preventDefault();try{document.execCommand('cut')}catch(_){}}else if(c==='KeyA'){e.preventDefault();var el=document.activeElement;if(_dozEdit(el)){if(el.setSelectionRange)el.setSelectionRange(0,el.value?el.value.length:0);else try{document.execCommand('selectAll')}catch(_){}}else{var g=window.getSelection&&window.getSelection();g&&g.selectAllChildren(document);}}},true);})();",
        );
        if options.flyfish {
            // Flyfish host 明确关闭了自带 toolbar，因此页面内没有可聚焦
            // 的搜索框。WebView 聚焦时直接把 Cmd/Ctrl+F 交还给产品，
            // 由统一 Find 条收 query，再调用 Flyfish `searchDocument`。
            script.push_str(
                "document.addEventListener('keydown',function(e){if((e.metaKey||e.ctrlKey)&&e.code==='KeyF'){e.preventDefault();e.stopPropagation();window.ipc.postMessage('find_preview');}},true);",
            );
        } else {
            // 浏览器/隔离 HTML 等普通页面仍优先使用页面自己的搜索框，
            // 找不到时退回平台原生页内查找。
            script.push_str(
                "(function(){function _dozPick(root){var best=null;function walk(n){if(!n||n.nodeType!==1&&n.nodeType!==9&&n.nodeType!==11)return;var list=n.querySelectorAll?n.querySelectorAll('input[type=search]'):[];for(var i=0;i<list.length;i++){var el=list[i];if(el.disabled||el.readOnly)continue;if(!best)best=el;}if(!best){var any=n.querySelectorAll?n.querySelectorAll('input[type=text],input:not([type])'):[];for(var j=0;j<any.length;j++){var a=any[j];if(a.disabled||a.readOnly)continue;var box=a.closest&&(a.closest('[role=search]')||a.closest('.file-viewer-web-search')||a.closest('form[role=search]'));if(box){best=a;break;}}}var hosts=n.querySelectorAll?n.querySelectorAll('*'):[];for(var k=0;k<hosts.length;k++){var sr=hosts[k].shadowRoot;if(sr)walk(sr);}}walk(root||document);return best;}document.addEventListener('keydown',function(e){if((e.metaKey||e.ctrlKey)&&e.code==='KeyF'){e.preventDefault();var box=_dozPick(document);if(box){try{box.focus();if(box.select)box.select();}catch(_){}window.ipc.postMessage('find_page');}else{window.ipc.postMessage('find_native');}}},true);})();",
            );
        }
    }
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 搬迁前在 `dozer-app::runtime` 现状上生成的黄金:四个组合逐字节相同。
    #[test]
    fn golden_matches_the_pre_migration_script() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/preview_host_script_golden.txt"
        );
        let want = std::fs::read_to_string(path).expect("缺少注入脚本黄金");
        let mut got = String::new();
        for (label, text_editing, flyfish) in [
            ("editor_some / page", false, false),
            ("editor_some / flyfish", false, true),
            ("editor_none / page", true, false),
            ("editor_none / flyfish", true, true),
        ] {
            got.push_str(&format!(
                "### {label}\n{}\n\n",
                preview_host_script(HostScriptOptions {
                    text_editing,
                    flyfish,
                })
            ));
        }
        assert!(got == want, "注入脚本与搬迁前黄金不一致");
    }

    #[test]
    fn every_variant_starts_with_focus_drag_and_zoom() {
        for (text_editing, flyfish) in [(true, true), (true, false), (false, false)] {
            let s = preview_host_script(HostScriptOptions {
                text_editing,
                flyfish,
            });
            assert!(s.starts_with(
                "document.addEventListener('mousedown',function(){window.ipc.postMessage('focus')},true);"
            ));
            assert!(s.contains("postMessage('zoom_in')"));
            assert!(s.contains("postMessage('zoom_out')"));
            assert!(s.contains("postMessage('zoom_reset')"));
            assert!(s.contains("postMessage('mouseup')"));
        }
    }

    #[test]
    fn text_editing_segment_is_absent_for_editor_hosts() {
        let s = preview_host_script(HostScriptOptions {
            text_editing: false,
            flyfish: false,
        });
        assert!(!s.contains("__dozerPaste"));
        assert!(!s.contains("KeyC"));
        assert!(!s.contains("find_page"));
        assert!(!s.contains("find_preview"));
    }

    #[test]
    fn flyfish_takes_find_while_other_hosts_pick_the_page_search_box() {
        let flyfish = preview_host_script(HostScriptOptions {
            text_editing: true,
            flyfish: true,
        });
        assert!(flyfish.contains("postMessage('find_preview')"));
        assert!(!flyfish.contains("find_page"));

        let page = preview_host_script(HostScriptOptions {
            text_editing: true,
            flyfish: false,
        });
        assert!(page.contains("postMessage('find_page')"));
        assert!(page.contains("postMessage('find_native')"));
        assert!(!page.contains("find_preview"));
        assert!(page.contains("__dozerPaste"));
    }
}
