import importlib.util, os, unittest
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("cu", os.path.join(HERE, "client_uses.py"))
cu = importlib.util.module_from_spec(spec); spec.loader.exec_module(cu)

class Scan(unittest.TestCase):
    NAMES = {"get_usage_summary", "list_todos"}

    def test_same_line_and_multiline_calls_are_found(self):
        text = "let a = client.list_todos(1).await;\nlet b = client\n    .get_usage_summary(&cwd, None)\n    .await;\n"
        self.assertEqual(cu.methods_used(text, self.NAMES), {"list_todos", "get_usage_summary"})

    def test_comments_and_strings_are_ignored(self):
        text = '// client.list_todos(1)\nlet s = "client.get_usage_summary(";\n'
        self.assertEqual(cu.methods_used(text, self.NAMES), set())

    def test_unknown_names_are_ignored(self):
        self.assertEqual(cu.methods_used("x.other(1)", self.NAMES), set())

if __name__ == "__main__":
    unittest.main()
