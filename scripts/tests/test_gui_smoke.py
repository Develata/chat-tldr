"""Negative controls for the GUI release evidence checks."""
import argparse
import hashlib
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "release"))
import gui_smoke


class GuiSmokeTests(unittest.TestCase):
    def archive(self, root, extra=None, omit_gui=False, omit_manager=False):
        archive = root / "gui.zip"
        prefix = "chat-tldr-gui-0.1.0-windows-x86_64/"
        names = ["chat-tldr.exe", "chat-tldr-gui.exe", "chat-tldr-qce-manager.exe", "apps/qce-manager/README.md", "LICENSE", "README.md",
                 "config.example.toml", "docs/DOCKER.md", "docs/RELEASING.md", "apps/gui/README.md",
                 "apps/gui/assets/fonts/OFL.txt", "apps/gui/assets/fonts/README.md"]
        if extra:
            names.append(extra)
        if omit_gui:
            names.remove("chat-tldr-gui.exe")
        if omit_manager:
            names.remove("chat-tldr-qce-manager.exe")
        with zipfile.ZipFile(archive, "x") as output:
            for name in names:
                output.writestr(prefix + name, b"synthetic")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        archive.with_name("gui.zip.sha256").write_text(f"{digest}  gui.zip\n", encoding="utf-8")
        return argparse.Namespace(archive=archive, version="0.1.0")

    def test_tampered_bundle_fails_before_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            options = self.archive(Path(directory))
            with options.archive.open("ab") as stream:
                stream.write(b"corruption")
            with self.assertRaisesRegex(RuntimeError, "checksum"):
                with gui_smoke.binaries(options):
                    self.fail("tampered archive accepted")

    def test_private_and_traversal_entries_fail_even_with_valid_checksum(self):
        for name in (".env", "../../outside.txt"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                options = self.archive(Path(directory), name)
                with self.assertRaisesRegex(RuntimeError, "public file list"):
                    with gui_smoke.binaries(options):
                        self.fail("unexpected file accepted")

    def test_missing_gui_cannot_pass_using_only_the_cli(self):
        with tempfile.TemporaryDirectory() as directory:
            options = self.archive(Path(directory), omit_gui=True)
            with self.assertRaisesRegex(RuntimeError, "public file list"):
                with gui_smoke.binaries(options):
                    self.fail("incomplete package accepted")

    def test_truncated_capture_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "capture.png"
            path.write_bytes(b"\x89PNG\r\n\x1a\n")
            with self.assertRaisesRegex(RuntimeError, "missing or undersized"):
                gui_smoke.png_size(path)

    def test_missing_manager_cannot_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            options = self.archive(Path(directory), omit_manager=True)
            with self.assertRaisesRegex(RuntimeError, "public file list"):
                with gui_smoke.binaries(options):
                    self.fail("GUI without manager accepted")


if __name__ == "__main__":
    unittest.main()
