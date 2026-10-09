import importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("unit_edges", os.path.join(os.path.dirname(__file__), "unit_edges.py"))
unit_edges = importlib.util.module_from_spec(spec); spec.loader.exec_module(unit_edges)


class Classify(unittest.TestCase):
    def test_contract_types_are_A(self):
        self.assertEqual(unit_edges.classify("panel_host", "PanelHost"), "A")
        self.assertEqual(unit_edges.classify("panel_host", "ActivationCtx"), "A")

    def test_panel_host_testing_is_T(self):
        self.assertEqual(unit_edges.classify("panel_host", "testing"), "T")

    def test_generic_ui_is_B_and_domain_is_C_and_service_is_S(self):
        self.assertEqual(unit_edges.classify("chrome", "homespace"), "B")
        self.assertEqual(unit_edges.classify("app", "HoverId"), "B")
        self.assertEqual(unit_edges.classify("workspace", "agent_icon"), "B")
        self.assertEqual(unit_edges.classify("conversation", "ConversationMeta"), "C")
        self.assertEqual(unit_edges.classify("dozer_core", "protocol"), "C")
        self.assertEqual(unit_edges.classify("dozer_model", "AgentKind"), "C")
        self.assertEqual(unit_edges.classify("dozer_client", "Client"), "S")
        self.assertEqual(unit_edges.classify("bytegit", "Repo"), "S")

    def test_unknown_is_question_mark(self):
        self.assertEqual(unit_edges.classify("nonexistent", "Thing"), "?")


class ScanUsage(unittest.TestCase):
    def test_usage_has_no_unclassified_edges(self):
        rows = unit_edges.scan("ext:usage")
        self.assertTrue(rows, "ext:usage 应至少有一条依赖边")
        self.assertEqual([r for r in rows if r[0] == "?"], [])

    def test_usage_depends_on_workspace_helpers(self):
        mods = {(r[1], r[2]) for r in unit_edges.scan("ext:usage")}
        self.assertIn(("workspace", "agent_icon"), mods)


if __name__ == "__main__":
    unittest.main()
