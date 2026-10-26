//! Embedded group-chat web assets and the neutral webview spec.

pub const HOST_HTML: &[u8] = include_bytes!("../assets/group-chat-content/host.html");
pub const CONTENT_JS: &[u8] = include_bytes!("../assets/group-chat-content/group-chat-content.js");
pub const CONTENT_CSS: &[u8] =
    include_bytes!("../assets/group-chat-content/group-chat-content.css");

/// Serve one of the three trusted files. Only exact names match; traversal and
/// unknown paths return `None` (the host maps that to 404).
pub fn serve_group_chat_content(path: &str) -> Option<(&'static str, &'static [u8])> {
    match path {
        "host.html" | "/host.html" => Some(("text/html", HOST_HTML)),
        "group-chat-content.js" | "/group-chat-content.js" => Some(("text/javascript", CONTENT_JS)),
        "group-chat-content.css" | "/group-chat-content.css" => Some(("text/css", CONTENT_CSS)),
        _ => None,
    }
}

/// Webview spec for the content pane. `None` means the native failure shell owns
/// the pane. `park` keeps the view alive but off-screen until the page is ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupChatWebviewSpec {
    pub url: String,
    pub visible: bool,
    pub park_offscreen: bool,
}

pub fn group_chat_webview_spec(
    failed: bool,
    loaded_or_ready: bool,
    scheme: &str,
    theme: &str,
) -> Option<GroupChatWebviewSpec> {
    if failed {
        return None;
    }
    Some(GroupChatWebviewSpec {
        url: format!("{scheme}://group-chat-content/host.html?theme={theme}"),
        visible: true,
        park_offscreen: !loaded_or_ready,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_model::AgentKind;

    #[test]
    fn bundle_assets_are_present_and_strict_csp() {
        for (name, bytes) in [
            ("host.html", HOST_HTML),
            ("group-chat-content.js", CONTENT_JS),
            ("group-chat-content.css", CONTENT_CSS),
        ] {
            assert!(!bytes.is_empty(), "{name}");
        }
        let html = std::str::from_utf8(HOST_HTML).unwrap();
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"));
        assert!(!html.contains("http://") && !html.contains("https://"));
    }

    #[test]
    fn serve_serves_the_three_files_without_traversal_or_unknown_paths() {
        for path in [
            "host.html",
            "group-chat-content.js",
            "group-chat-content.css",
        ] {
            assert!(serve_group_chat_content(path).is_some(), "{path}");
        }
        for path in ["../usage-content/host.html", "nope", "", "/../host.html"] {
            assert!(serve_group_chat_content(path).is_none(), "{path}");
        }
    }

    #[test]
    fn webview_spec_url_uses_the_given_scheme_and_theme() {
        let spec = group_chat_webview_spec(false, true, "dozer", "dark").unwrap();
        assert_eq!(spec.url, "dozer://group-chat-content/host.html?theme=dark");
        assert!(spec.visible);
        assert!(!spec.park_offscreen);
    }

    #[test]
    fn webview_spec_is_none_when_failed_and_parked_until_ready() {
        assert!(group_chat_webview_spec(true, false, "dozer", "dark").is_none());
        let spec = group_chat_webview_spec(false, false, "dozer", "dark").unwrap();
        assert!(spec.park_offscreen);
    }

    #[test]
    fn creatable_agents_are_exactly_claude_codex_codebuddy_opencode_qodercn() {
        assert_eq!(
            crate::consts::CREATABLE_AGENTS,
            &[
                AgentKind::Claude,
                AgentKind::Codex,
                AgentKind::Codebuddy,
                AgentKind::Opencode,
                AgentKind::Qodercn,
            ]
        );
    }
}
