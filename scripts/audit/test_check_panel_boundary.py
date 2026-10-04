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
class PanePick(unittest.TestCase):
    """R-PANE-PICK:全库里手写"按 PanelKind::Project 选预览窗格"的写法只许减不许增(H1 切片 1 的回退防线)。"""
    def scan1(self, rel, text):
        return g.scan({rel: text}).get(rel, {})
    def test_if_eq_project_counts_anywhere(self):
        self.assertEqual(self.scan1("workspace/state.rs", "if kind == PanelKind::Project { a } else { b }"), {"R-PANE-PICK": 1})
    def test_match_arm_selecting_project_preview_counts(self):
        text = "match kind { PanelKind::Project => &mut ws.project_preview, _ => &mut ws.preview }"
        self.assertEqual(self.scan1("app/update.rs", text), {"R-PANE-PICK": 1})
    def test_other_project_arms_do_not_count(self):
        self.assertEqual(self.scan1("chrome/rail.rs", 'PanelKind::Project => (icon, "项目"),'), {})
    def test_comments_do_not_count(self):
        self.assertEqual(self.scan1("a.rs", "// if kind == PanelKind::Project {"), {})
class EnumVariants(unittest.TestCase):
    """R-HOVERID-VARIANTS / R-HOVERSLOT-VARIANTS:宿主的 HoverId 与通用槽位词汇 HoverSlot 的变体数只许减不许增
    (H2:面板按钮不得再写回 host 枚举;要新增悬停元素先用 `HoverId::named(panel, ..)`)。"""
    ENUM = "pub enum HoverId {\n    /// doc\n    Topbar(TopbarButton),\n    Rail(R),\n    HomeTab,\n    // c\n    Panel(PanelKind, HoverSlot),\n}\n"
    def test_counts_variants_not_docs_or_comments(self):
        got = g.scan({"app/state.rs": self.ENUM}).get("app/state.rs", {})
        self.assertEqual(got, {"R-HOVERID-VARIANTS": 4})
    def test_slot_enum_counted_in_panel_host(self):
        text = "pub enum HoverSlot {\n    ListCollapse,\n    TabItem(u64),\n    Named(&'static str),\n}\n"
        got = g.scan({"panel_host.rs": text}).get("panel_host.rs", {})
        self.assertEqual(got, {"R-HOVERSLOT-VARIANTS": 3})
    def test_other_files_ignore_same_named_enums(self):
        self.assertEqual(g.scan({"extensions/x.rs": self.ENUM}).get("extensions/x.rs", {}), {})
class HostPanelCoupling(unittest.TestCase):
    """R-HOST-PANEL-ARMS / R-HOST-EXECUTORS:host 里"替面板做事"的代码只许减不许增(H4)。"""
    def scan1(self, rel, text):
        return g.scan({rel: text}).get(rel, {})
    def test_panel_specific_arms_in_app_update_are_counted(self):
        text = (
            "            Message::Files(files::Message::CopyPath(p, k)) => {}\n"
            "            Message::GitLog(msg) => {}\n"
            "            Message::TermInput(x) => {}\n"          # 非面板消息
            "                Message::Files(inner) => {}\n"       # 缩进更深:不是顶层臂
        )
        self.assertEqual(self.scan1("app/update.rs", text), {"R-HOST-PANEL-ARMS": 2})
    def test_arms_in_other_files_are_not_counted(self):
        text = "            Message::Files(files::Message::CopyPath(p, k)) => {}\n"
        self.assertEqual(self.scan1("app/view.rs", text), {})
    def test_commented_out_arms_are_not_counted(self):
        text = "            // Message::Files(x) => {}\n"
        self.assertEqual(self.scan1("app/update.rs", text), {})
    def test_per_panel_effect_executors_in_host_are_counted(self):
        text = "    fn run_group_chat_effects(&mut self) {}\n    fn run_other_effects(&mut self) {}\n    fn run_effect(&mut self) {}\n"
        self.assertEqual(self.scan1("app/update.rs", text), {"R-HOST-EXECUTORS": 2})
    def test_executors_outside_app_are_not_counted(self):
        text = "pub fn run_group_chat_effects() {}\n"
        self.assertEqual(self.scan1("extensions/group_chat/mod.rs", text), {})
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
