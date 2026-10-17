"""Tests for wait.sh: run with `python3 .claude/skills/gpui-mockup/scripts/test_wait.py`."""

from pathlib import Path
import subprocess
import tempfile
import threading
import time
import unittest

SCRIPT = Path(__file__).with_name("wait.sh")


class WaitTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="mockup-wait-")
        self.addCleanup(self.directory.cleanup)
        self.feedback = Path(self.directory.name) / "feedback.md"

    def run_wait(self, *args):
        return subprocess.run(
            ["bash", str(SCRIPT), str(self.feedback), *args],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )

    def test_zero_without_notes_exits_4(self):
        result = self.run_wait("0")
        self.assertEqual(result.returncode, 4)
        self.assertIn("no feedback", result.stderr)

    def test_prints_and_consumes_each_send(self):
        self.feedback.write_text("Picked: B\n")
        first = self.run_wait("0")
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(first.stdout, "Picked: B\n")
        self.assertFalse(self.feedback.exists())
        self.assertEqual(Path(f"{self.feedback}.1").read_text(), "Picked: B\n")

        self.feedback.write_text("Picked: C\n")
        second = self.run_wait("0")
        self.assertEqual(second.stdout, "Picked: C\n")
        self.assertTrue(Path(f"{self.feedback}.2").exists())

    def test_waits_for_a_later_send(self):
        def send():
            time.sleep(0.5)
            self.feedback.write_text("Picked: A\n")

        sender = threading.Thread(target=send)
        sender.start()
        result = self.run_wait("5")
        sender.join()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "Picked: A\n")

    def test_rejects_bad_arguments(self):
        self.assertEqual(self.run_wait("soon").returncode, 2)
        self.assertEqual(
            subprocess.run(["bash", str(SCRIPT)], capture_output=True, check=False).returncode, 2
        )


if __name__ == "__main__":
    unittest.main()
