"""Exercise the host-dependency gate against real Cargo graphs and command failures."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "check-host-deps.sh"
CARGO = shutil.which("cargo")


class HostDepsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="host-deps-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        shutil.copy2(SOURCE, self.root / "scripts/check-host-deps.sh")
        self.env = dict(os.environ, CARGO_NET_OFFLINE="true",
                        CARGO_TARGET_DIR=str(self.root / "target"))

    def run_gate(self):
        return subprocess.run(["bash", "scripts/check-host-deps.sh"], cwd=self.root,
                              env=self.env, capture_output=True, text=True, timeout=30)

    def workspace(self, name="dozer-payments", dep=None, optional=False, section="dependencies"):
        (self.root / "Cargo.toml").write_text('[workspace]\nresolver="3"\nmembers=["app"]\nexclude=["fake"]\n')
        app = self.root / "app"
        (app / "src").mkdir(parents=True)
        (app / "src/lib.rs").write_text("")
        manifest = f'[package]\nname="{name}"\nversion="0.0.0"\nedition="2024"\n'
        if dep:
            fake = self.root / "fake"
            (fake / "src").mkdir(parents=True)
            (fake / "src/lib.rs").write_text("")
            (fake / "Cargo.toml").write_text(
                f'[package]\nname="{dep}"\nversion="0.0.0"\nedition="2024"\n[workspace]\n')
            opt = ", optional=true" if optional else ""
            manifest += f'[{section}]\n{dep}={{path="../fake"{opt}}}\n'
        (app / "Cargo.toml").write_text(manifest)

    def test_new_business_members_are_checked(self):
        self.workspace(dep="wry")
        result = self.run_gate()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("业务 crate dozer-payments", result.stderr)

    def test_optional_dependencies_are_checked(self):
        self.workspace(name="dozer-model", dep="iced", optional=True)
        result = self.run_gate()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("iced", result.stderr)

    def test_build_dependencies_are_checked(self):
        self.workspace(dep="bytehost-probe", section="build-dependencies")
        result = self.run_gate()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("bytehost-probe", result.stderr)

    def test_target_dependencies_are_checked(self):
        self.workspace(dep="wry", section="target.'cfg(target_os = \"windows\")'.dependencies")
        result = self.run_gate()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("业务 crate dozer-payments", result.stderr)

    def test_legal_business_dependency_passes(self):
        self.workspace(dep="neutral-probe")
        result = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("1 个业务 crate", result.stdout)

    def test_explicit_host_adapter_is_allowed(self):
        self.workspace(name="dozer-panel-host", dep="wry")
        result = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def failing_cargo(self, command):
        self.workspace()
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        cargo = bin_dir / "cargo"
        cargo.write_text(f'#!/bin/sh\nif [ "$1" = "{command}" ]; then\n'
                         '  echo "injected cargo failure" >&2\n  exit 101\nfi\n'
                         f'exec "{CARGO}" "$@"\n')
        cargo.chmod(0o755)
        self.env["PATH"] = str(bin_dir) + os.pathsep + self.env["PATH"]
        result = self.run_gate()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("host-deps check: ok", result.stdout)

    def test_tree_failure_is_not_success(self):
        self.failing_cargo("tree")

    def test_metadata_failure_is_not_success(self):
        self.failing_cargo("metadata")


if __name__ == "__main__":
    unittest.main()
