"""`scan_sources.py` 提取器的回归样例(Task 12)。

覆盖给门禁"声称能发现"的每种形态钉上样例:普通/raw 字符串、`format!`、
`#[cfg(test)]` 边界、字符串感知的注释剥离、字符字面量、JSX 文本、HTML 属性与
多行模板——避免"用简单中文 grep 就宣称无遗漏"。
"""
import importlib.util
import os
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("scan_sources", os.path.join(_HERE, "scan_sources.py"))
scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scan)


def extract_rust(text, rel="crates/dozer-app/src/x.rs"):
    recs = []
    scan.extract_rust(text, rel, recs)
    return [r["text"] for r in recs]


def extract_js(text, rel="crates/dozer-app/web/x/src/main.ts"):
    recs = []
    scan.extract_js(text, rel, recs)
    return [r["text"] for r in recs]


def extract_html(text, rel="crates/dozer-app/src/html_host.html"):
    recs = []
    scan.extract_html(text, rel, recs)
    return [r["text"] for r in recs]


class RustStrings(unittest.TestCase):
    def test_plain_string_recorded(self):
        self.assertIn("保存", extract_rust('let x = "保存";'))

    def test_raw_string_recorded(self):
        self.assertIn("原始 文本", extract_rust('let x = r#"原始 文本"#;'))

    def test_comment_not_recorded(self):
        self.assertEqual(extract_rust("// 这是注释 中文\nlet x = 1;"), [])

    def test_block_comment_not_recorded(self):
        self.assertEqual(extract_rust("/* 块注释 中文 */\nlet x = 1;"), [])

    def test_url_string_does_not_desync_following_string(self):
        # 含 `//` 的 URL 字符串本身被"泄漏防护"丢弃(见 _maybe_record:含 `//`
        # 判为引号错位伪候选),但**不能吞掉其后**的正常 UI 文案。
        got = extract_rust('let u = "dozer://html/x";\nlet y = "界面";')
        self.assertIn("界面", got)

    def test_char_literal_with_quote_does_not_desync(self):
        # `trim_matches('"')` 里的 `"` 不能吞掉后面的字符串(ssh.rs 实证 bug)。
        got = extract_rust("let s = t.trim_matches('\"');\nlet y = \"界面\";")
        self.assertIn("界面", got)

    def test_test_context_excluded_by_classify(self):
        text = '#[cfg(test)]\nmod tests {\n  fn f() { let x = "用例"; }\n}\n'
        recs = []
        scan.extract_rust(text, "crates/dozer-app/src/x.rs", recs)
        # 提取器本身仍抽取;分类阶段才排除。
        self.assertIn("用例", [r["text"] for r in recs])
        r = recs[0]
        usage, _ = scan.classify(r["path"], text, r["offset"], r["kind"])
        self.assertEqual(usage, "test")


class JsStrings(unittest.TestCase):
    def test_plain_template_recorded(self):
        self.assertIn("你好世界", extract_js("const s = `你好世界`;"))

    def test_multiline_template_dropped_by_leak_guard(self):
        # 多行候选被判为引号错位伪候选而丢弃(真实 UI 文案是单行)。
        self.assertEqual(extract_js("const s = `第一行\n第二行`;"), [])

    def test_line_comment_not_recorded(self):
        self.assertEqual(extract_js("// 注释 中文\nconst x = 1;"), [])

    def test_comment_marker_string_does_not_desync_following(self):
        got = extract_js('const u = "http://x/y";\nconst v = "保存";')
        self.assertIn("保存", got)

    def test_jsx_text_node(self):
        self.assertIn("你好", [t.strip() for t in extract_js("<div>你好</div>")])


class HtmlStrings(unittest.TestCase):
    def test_attr_and_text(self):
        html = '<button title="保存" aria-label="打开文件">提交</button>'
        got = extract_html(html)
        self.assertIn("保存", got)
        self.assertIn("打开文件", got)
        self.assertIn("提交", got)

    def test_html_comment_not_recorded(self):
        self.assertEqual(extract_html("<!-- 文档注释 中文 -->\n<div>ok</div>"), [])

    def test_script_style_stripped(self):
        html = '<script>var s = "脚本中文";</script><style>/* 样式中文 */</style>'
        self.assertEqual(extract_html(html), [])


if __name__ == "__main__":
    unittest.main()
