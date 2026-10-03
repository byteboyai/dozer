import importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("g", os.path.join(os.path.dirname(__file__), "check_panel_boundary.py"))
g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)

class Scan(unittest.TestCase):
    def test_only_extensions_files_count(self):
        files = {"extensions/todo/view.rs": "use crate::app::{App, HoverId};\nuse crate::workspace::Workspace;",
                 "app/update.rs": "use crate::app::App;",
                 "extensions/ssh.rs": "use crate::theme;"}
        self.assertEqual(g.scan(files), {"extensions/todo/view.rs": {"R-APP": 1, "R-WS": 1}})
class UseCounting(unittest.TestCase):
    """门禁数的是 App/Workspace 的使用次数,不是 import 路径条数(评审 Important 1)。"""
    def scan1(self, text):
        return g.scan({"extensions/x.rs": text}).get("extensions/x.rs", {})
    def test_params_count_not_just_the_import(self):
        text = "use crate::app::App;\nfn a(app: &App) {}\nfn b(app: &App) {}"
        self.assertEqual(self.scan1(text), {"R-APP": 3})
    def test_module_import_then_qualified_use(self):
        self.assertEqual(self.scan1("use crate::app;\nfn f(a: &app::App) {}"), {"R-APP": 1})
    def test_fully_qualified_path_without_import(self):
        self.assertEqual(self.scan1("fn f(a: &crate::app::App) {}"), {"R-APP": 1})
    def test_super_super_path(self):
        self.assertEqual(self.scan1("use super::super::app::App;\nfn f(a: &App) {}"), {"R-APP": 2})
    def test_alias_import(self):
        self.assertEqual(self.scan1("use crate::app::App as Host;\nfn f(a: &Host) {}"), {"R-APP": 2})
    def test_unrelated_app_word_without_import_not_counted(self):
        self.assertEqual(self.scan1('let s = "用外部 App 打开";'), {})
    def test_app_word_in_string_not_counted_when_imported(self):
        self.assertEqual(self.scan1('use crate::app::App;\nlet s = "App";'), {"R-APP": 1})
    def test_workspace_params(self):
        text = "use crate::workspace::Workspace;\nfn f(w: &Workspace) {}"
        self.assertEqual(self.scan1(text), {"R-WS": 2})
class Compare(unittest.TestCase):
    def test_decrease_and_equal_pass(self):
        base = {"extensions/a.rs": {"R-APP": 3}}
        self.assertEqual(g.compare(base, {"extensions/a.rs": {"R-APP": 3}}), [])
        self.assertEqual(g.compare(base, {"extensions/a.rs": {"R-APP": 1}}), [])
        self.assertEqual(g.compare(base, {}), [])
    def test_increase_and_new_file_fail(self):
        base = {"extensions/a.rs": {"R-APP": 1}}
        cur = {"extensions/a.rs": {"R-APP": 2}, "extensions/b.rs": {"R-WS": 1}}
        self.assertEqual(g.compare(base, cur), [("extensions/a.rs", "R-APP", 1, 2), ("extensions/b.rs", "R-WS", 0, 1)])
if __name__ == "__main__":
    unittest.main()
