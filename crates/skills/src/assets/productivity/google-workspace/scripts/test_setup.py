import importlib.util
import os
import stat
import tempfile
import unittest
from pathlib import Path


def load_script(name):
    path = Path(__file__).with_name(name)
    spec = importlib.util.spec_from_file_location(f"moltis_{path.stem}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_setup():
    return load_script("setup.py")


class GoogleSetupTest(unittest.TestCase):
    def test_gmail_readonly_uses_only_read_scope(self):
        setup = load_setup()
        self.assertEqual(
            setup.selected_scopes("gmail-readonly"),
            ["https://www.googleapis.com/auth/gmail.readonly"],
        )

    def test_data_dir_precedes_legacy_home(self):
        setup = load_setup()
        previous_data = os.environ.get("MOLTIS_DATA_DIR")
        previous_legacy = os.environ.get("HERMES_HOME")
        try:
            os.environ["MOLTIS_DATA_DIR"] = "/tmp/moltis-data"
            os.environ["HERMES_HOME"] = "/tmp/hermes-data"
            self.assertEqual(setup.get_moltis_home(), Path("/tmp/moltis-data"))
        finally:
            if previous_data is None:
                os.environ.pop("MOLTIS_DATA_DIR", None)
            else:
                os.environ["MOLTIS_DATA_DIR"] = previous_data
            if previous_legacy is None:
                os.environ.pop("HERMES_HOME", None)
            else:
                os.environ["HERMES_HOME"] = previous_legacy

    @unittest.skipIf(os.name == "nt", "POSIX file modes are not available")
    def test_private_json_uses_owner_only_permissions(self):
        setup = load_setup()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nested" / "token.json"
            setup._write_private_json(path, {"token": "secret"})
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    @unittest.skipIf(os.name == "nt", "symbolic link semantics differ on Windows")
    def test_private_json_replaces_symlink_without_following_it(self):
        setup = load_setup()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            victim = root / "victim.json"
            victim.write_text("unchanged")
            path = root / "token.json"
            path.symlink_to(victim)

            setup._write_private_json(path, {"token": "secret"})

            self.assertFalse(path.is_symlink())
            self.assertEqual(victim.read_text(), "unchanged")

    @unittest.skipIf(os.name == "nt", "POSIX file modes are not available")
    def test_every_token_writer_is_atomic_and_private(self):
        for script in ("setup.py", "google_api.py", "gws_bridge.py"):
            with self.subTest(script=script), tempfile.TemporaryDirectory() as directory:
                module = load_script(script)
                path = Path(directory) / "token.json"
                module._write_private_json(path, {"token": "secret"})
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)


if __name__ == "__main__":
    unittest.main()
