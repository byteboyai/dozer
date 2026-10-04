import importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("edges", os.path.join(os.path.dirname(__file__), "edges.py"))
edges = importlib.util.module_from_spec(spec); spec.loader.exec_module(edges)

class Flatten(unittest.TestCase):
    def test_nested_and_alias(self):
        self.assertEqual(edges.flatten("a::{B, c::{D, E as F}, self}"), ["a::B", "a::c::D", "a::c::E", "a::self"])
class Edges(unittest.TestCase):
    def test_multiline_use_and_inline(self):
        got = dict(edges.edges_of("use crate::app::{\n  App, HoverId,\n};\nlet x = crate::theme::color();"))
        self.assertEqual(got, {("app", "App"): 1, ("app", "HoverId"): 1, ("theme", "color"): 1})
    def test_comments_ignored(self):
        self.assertEqual(dict(edges.edges_of("// use crate::app::App;\n/* crate::theme::x */")), {})
    def test_super_not_counted(self):
        self.assertEqual(dict(edges.edges_of("use super::*; use self::a::B;")), {})
    def test_unit_of(self):
        u = edges.unit_of
        self.assertEqual(u("extensions/todo/view.rs"), "ext:todo")
        self.assertEqual(u("extensions/git_log.rs"), "ext:git_log")
        self.assertEqual(u("app/update.rs"), "layer:app")
        self.assertEqual(u("secrets.rs"), "root:secrets")
if __name__ == "__main__":
    unittest.main()
