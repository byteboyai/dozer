"""`check_migration.py` 门禁的单元测试(Task 12)。

分两层:
  - 纯函数层:直接对 `find_missing_keys` / `find_unclassified` / fallback 正则
    喂合成输入,钉住"缺 key / 中文 fallback / 未分类"三类必须报错。
  - 注入层:往临时源码树写一段 Rust,真跑 `extract_key_refs` +
    `find_missing_keys` + `check_english_fallbacks`,验证生产源里的
    (1) 新硬编码中文 UI、(2) 缺 key、(3) 中文 fallback 都会被拦下。
"""
import importlib.util
import os
import shutil
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("check_migration", os.path.join(_HERE, "check_migration.py"))
cm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cm)


class FindMissingKeys(unittest.TestCase):
    def test_present_ok(self):
        bundles = {"dozer.host": {"en": {"a": "A"}, "zh-CN": {"a": "甲"}}}
        self.assertEqual(cm.find_missing_keys({("dozer.host", "a"): ["x"]}, bundles), [])

    def test_missing_in_one_locale_fails(self):
        bundles = {"dozer.host": {"en": {"a": "A"}, "zh-CN": {}}}
        probs = cm.find_missing_keys({("dozer.host", "a"): ["x.rs"]}, bundles)
        self.assertEqual(len(probs), 1)
        self.assertIn("locale=zh-CN", probs[0])
        self.assertIn("key=a", probs[0])

    def test_unknown_owner_fails(self):
        probs = cm.find_missing_keys({("dozer.nope", "a"): ["x.rs"]}, {})
        self.assertEqual(len(probs), 1)
        self.assertIn("未知 owner", probs[0])

    def test_injected_new_key_absent_from_bundle_fails(self):
        # 生产源新增一个从未入包的 key。
        bundles = {"dozer.host": {"en": {}, "zh-CN": {}}}
        probs = cm.find_missing_keys({("dozer.host", "brand.new"): ["y.rs"]}, bundles)
        self.assertEqual(len(probs), 2)  # en + zh-CN 各一条


class FindUnclassified(unittest.TestCase):
    def test_hardcoded_zh_candidate_fails(self):
        recs = [{"path": "a.rs", "text": "保存", "status": "unresolved"}]
        self.assertEqual(len(cm.find_unclassified(recs)), 1)

    def test_classified_candidates_ok(self):
        recs = [
            {"path": "a.rs", "text": "保存", "status": "excluded"},
            {"path": "a.rs", "text": "Save", "status": "excluded"},
        ]
        self.assertEqual(cm.find_unclassified(recs), [])


class FallbackRegex(unittest.TestCase):
    def test_chinese_fallback_matched(self):
        code = 'let s = crate::i18n::host_text("k", "保存", locale);'
        fbs = [m.group(1) for m in cm.FALLBACK_CALL_RE.finditer(code)]
        self.assertIn("保存", fbs)

    def test_english_fallback_matched_and_clean(self):
        code = 'let s = crate::i18n::host_text("k", "Save", locale);'
        fbs = [m.group(1) for m in cm.FALLBACK_CALL_RE.finditer(code)]
        self.assertEqual(fbs, ["Save"])


class Injection(unittest.TestCase):
    """写一段真实 Rust 到临时树,跑抽取+门禁,确认硬编码/缺 key/中文 fallback 被拦。"""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="i18n-inj-")
        self.root = os.path.join(self.tmp, "crates", "dozer-app", "src")
        os.makedirs(self.root)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _write(self, name, text):
        with open(os.path.join(self.root, name), "w", encoding="utf-8") as f:
            f.write(text)

    def test_hardcoded_zh_is_unclassified(self):
        # 新写一条中文 UI 字面量,没走 keyed 包装 → 必须落 unresolved。
        text = 'fn f() { let s = "保存文件"; }\n'
        recs = []
        cm.scan.extract_rust(text, "crates/dozer-app/src/bad.rs", recs)
        usage, _ = cm.scan.classify("crates/dozer-app/src/bad.rs", text, recs[0]["offset"], recs[0]["kind"])
        rec = {"path": "crates/dozer-app/src/bad.rs", "text": "保存文件", "usage": usage}
        status, _reason = cm.build_inventory.classify_status(rec)
        self.assertEqual(status, "unresolved")

    def test_missing_key_detected_end_to_end(self):
        self._write("view.rs", 'fn f() { let s = crate::i18n::host_text("brand.new", "Brand New", locale); }\n')
        refs = cm.extract_key_refs({}, srcroots=[self.root])
        bundles = {"dozer.host": {"en": {}, "zh-CN": {}}}
        probs = cm.find_missing_keys(refs, bundles)
        self.assertTrue(any("brand.new" in p for p in probs))

    def test_english_fallback_clean_and_chinese_flagged(self):
        self._write("ok.rs", 'fn f() { let s = crate::i18n::host_text("k", "Save", locale); }\n')
        self.assertEqual(cm.check_english_fallbacks(srcroots=[self.root]), [])
        self._write("bad.rs", 'fn f() { let s = crate::i18n::host_text("k2", "保存", locale); }\n')
        probs = cm.check_english_fallbacks(srcroots=[self.root])
        self.assertEqual(len(probs), 1)
        self.assertIn("保存", probs[0])

    def test_test_context_key_not_counted(self):
        # 测试内的探针 key 不应被当成缺译。
        self._write("t.rs", '#[cfg(test)]\nmod tests {\n  fn f() { crate::i18n::host_keyed("does.not.exist", "x"); }\n}\n')
        refs = cm.extract_key_refs({}, srcroots=[self.root])
        self.assertNotIn(("dozer.host", "does.not.exist"), refs)


if __name__ == "__main__":
    unittest.main()
