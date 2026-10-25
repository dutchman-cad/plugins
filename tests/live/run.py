#!/usr/bin/env python3
"""Explicitly opted-in tests against a running stock SolidCraft."""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
PORT = os.getenv("SOLIDCRAFT_CONTROL_PORT")
TARGET = Path(os.getenv("CARGO_TARGET_DIR", str(Path(__file__).resolve().parents[2] / "target")))
BIN = os.getenv("DUTCHMAN_PLUGINS_BIN", str(TARGET / "release/dutchman-plugins"))

@unittest.skipUnless(PORT, "Set SOLIDCRAFT_CONTROL_PORT to a running stock app")
class Live(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.env = dict(os.environ, DUTCHMAN_PLUGINS_DATA=self.temp.name)

    def host(self, *args, input=None):
        run = subprocess.run([BIN, "--yes", "--port", PORT, *args], cwd=ROOT,
                             env=self.env, input=input, text=True, capture_output=True, timeout=180)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        return run.stdout

    def inspect(self):
        import sys
        if os.getenv("SOLIDCRAFT_CONTROL_DIR"):
            folder = Path(os.environ["SOLIDCRAFT_CONTROL_DIR"])
        elif sys.platform == "darwin":
            folder = Path.home() / "Library/Application Support/SolidCraft/control"
        elif os.name == "nt":
            folder = Path(os.environ["LOCALAPPDATA"]) / "SolidCraft/control"
        else:
            folder = Path(os.getenv("XDG_DATA_HOME", str(Path.home() / ".local/share"))) / "solidcraft/control"
        token = os.getenv("SOLIDCRAFT_CONTROL_TOKEN") or (folder / f"{PORT}.token").read_text().strip()
        with socket.create_connection(("127.0.0.1", int(PORT)), timeout=65) as sock:
            sock.sendall((json.dumps(dict(id=1, token=token, method="document.inspect", params={}))+ "\n").encode())
            reply = json.loads(sock.makefile("r").readline())
        self.assertTrue(reply["ok"], reply)
        return reply["result"]

    def test_import_synthetic_cube_and_open(self):
        path = Path(self.temp.name) / "cube.f3d"
        shutil.copyfile(ROOT / "examples/generated_unit_cube.f3d", path)
        self.host("import", "importer-fusion", str(path), "--open")
        self.assertEqual(len(self.inspect()["bodies"]), 1)

    def test_pilot_fillet(self):
        output = self.host("run", "pilot", "how", "round all edges 3 mm")
        result = json.loads(output.splitlines()[-1])
        self.assertTrue(result["matches"])
        self.assertIn("fillet", result["matches"][0]["command"]["id"])

    def test_panel_creates_box(self):
        before = len(self.inspect()["bodies"])
        self.host("panel", "hello-panel",
                  input='{"id":"make-box","inputs":{"size":12}}\nquit\n')
        self.assertEqual(len(self.inspect()["bodies"]), before + 1)

if __name__ == "__main__":
    unittest.main()
