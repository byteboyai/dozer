import importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("g", os.path.join(os.path.dirname(__file__), "check_panel_boundary.py"))
g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)

class Scan(unittest.TestCase):
    def test_only_extensions_files_count(self):
        files = {"extensions/todo/view.rs": "use crate::app::{App, HoverId};\nuse crate::workspace::Workspace;",
                 "app/update.rs": "use crate::app::App;",
                 "extensions/ssh.rs": "use crate::theme;"}
        self.assertEqual(g.scan(files), {"extensions/todo/view.rs": {"R-APP": 1, "R-WS": 1}})
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
