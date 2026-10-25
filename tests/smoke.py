#!/usr/bin/env python3
"""Offline end-to-end checks of the built reference plugins."""
import json
import os
from pathlib import Path
import shutil
import socketserver
import subprocess
import tempfile
import threading
import time
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = Path(os.getenv("CARGO_TARGET_DIR", str(Path(__file__).resolve().parents[1] / "target")))
BIN = os.getenv("DUTCHMAN_PLUGINS_BIN", str(TARGET / "release/dutchman-plugins"))

class Smoke(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.env = dict(os.environ, DUTCHMAN_PLUGINS_DATA=self.temp.name)

    def host(self, *args, input=None):
        run = subprocess.run([BIN, "--yes", *args], cwd=ROOT, env=self.env,
                             input=input, text=True, capture_output=True, timeout=180)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        return run.stdout

    def test_check_all_worlds(self):
        for plugin in ("pilot", "hello-panel", "importer-fusion", "importer-inventor", "importer-solidworks", "importer-freecad"):
            self.host("check", str(ROOT / "examples" / plugin))

    def test_importer_manifests_and_size_caps(self):
        families = {
            "fusion": ["f3d", "f3z"],
            "inventor": ["ipt", "iam"],
            "solidworks": ["sldprt"],
            "freecad": ["fcstd"],
        }
        for family, extensions in families.items():
            with self.subTest(family=family):
                source = ROOT / "plugins" / f"importer-{family}" / "plugin.json"
                folder = ROOT / "examples" / f"importer-{family}"
                manifest = json.loads((folder / "plugin.json").read_text())
                self.assertEqual(manifest, json.loads(source.read_text()))
                self.assertEqual(manifest["id"], f"importer-{family}")
                self.assertEqual([f["extension"] for f in manifest["formats"]], extensions)
                self.assertLessEqual((folder / manifest["component"]).stat().st_size, 16 * 1024 * 1024)

    def test_fusion_refuses_other_family(self):
        path = Path(self.temp.name) / "cube.ipt"
        shutil.copyfile(ROOT / "examples/generated_unit_cube.f3d", path)
        run = subprocess.run([BIN, "--yes", "import", "importer-fusion", str(path)],
                             cwd=ROOT, env=self.env, text=True, capture_output=True, timeout=180)
        self.assertNotEqual(run.returncode, 0)
        self.assertFalse(path.with_suffix(".solidcraft").exists())

    def test_pilot_how_and_plan(self):
        result = json.loads(self.host("run", "pilot", "how", "round all edges 3 mm").splitlines()[-1])
        self.assertIn("fillet", result["matches"][0]["command"]["id"])
        result = json.loads(self.host("run", "pilot", "plan", "make a 20 mm cube").splitlines()[-1])
        self.assertTrue(result.get("commands") or result.get("matches"))

    def test_import_synthetic_cube(self):
        path = Path(self.temp.name) / "cube.f3d"
        shutil.copyfile(ROOT / "examples/generated_unit_cube.f3d", path)
        self.host("import", "importer-fusion", str(path))
        with zipfile.ZipFile(path.with_suffix(".solidcraft")) as archive:
            doc = json.loads(archive.read("document.json"))
            self.assertTrue(doc["features"])
            self.assertTrue(any(name.endswith(".step") for name in archive.namelist()))

    def test_approval_is_required_and_changes_reprompt(self):
        folder = Path(self.temp.name) / "pilot"
        shutil.copytree(ROOT / "examples/pilot", folder)
        denied = subprocess.run([BIN, "run", str(folder), "how", "make a box"],
                                cwd=ROOT, env=self.env, input="n\n", text=True,
                                capture_output=True, timeout=60)
        self.assertNotEqual(denied.returncode, 0)
        self.assertIn("not approved", denied.stderr)
        self.host("run", str(folder), "how", "make a box")
        approved = subprocess.run([BIN, "run", str(folder), "how", "make a box"],
                                  cwd=ROOT, env=self.env, input="", text=True,
                                  capture_output=True, timeout=60)
        self.assertEqual(approved.returncode, 0, approved.stderr)
        manifest = folder / "plugin.json"
        value = json.loads(manifest.read_text())
        value["limits"] = {"memory_mib": 128}
        manifest.write_text(json.dumps(value))
        denied = subprocess.run([BIN, "run", str(folder), "how", "make a box"],
                                cwd=ROOT, env=self.env, input="n\n", text=True,
                                capture_output=True, timeout=60)
        self.assertNotEqual(denied.returncode, 0)

    def test_control_wait_obeys_guest_deadline(self):
        folder = Path(self.temp.name) / "panel"
        shutil.copytree(ROOT / "examples/hello-panel", folder)
        manifest = folder / "plugin.json"
        value = json.loads(manifest.read_text())
        value["limits"] = {"transaction_seconds": 1}
        manifest.write_text(json.dumps(value))
        release = threading.Event()
        dispatched = threading.Event()
        class Control(socketserver.StreamRequestHandler):
            def handle(self):
                while line := self.rfile.readline():
                    req = json.loads(line)
                    if req["method"] == "engine.execute":
                        dispatched.set()
                        release.wait(10)
                        return
                    self.wfile.write((json.dumps({"id": req["id"], "ok": True, "result": {"bodies": []}}) + "\n").encode())
        class Server(socketserver.ThreadingTCPServer):
            daemon_threads = True
            block_on_close = False
        with Server(("127.0.0.1", 0), Control) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            self.env["SOLIDCRAFT_CONTROL_TOKEN"] = "a" * 64
            try:
                process = subprocess.Popen([BIN, "--yes", "--port", str(server.server_address[1]),
                                            "run", str(folder), "box", '{"size":12}'],
                                           cwd=ROOT, env=self.env, text=True,
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                try:
                    if not dispatched.wait(90):
                        stdout, stderr = process.communicate(timeout=5)
                        self.fail("Engine command did not reach the mock: " + stdout + stderr)
                    started = time.monotonic()
                    stdout, stderr = process.communicate(timeout=3)
                    elapsed = time.monotonic() - started
                    run = subprocess.CompletedProcess(process.args, process.returncode, stdout, stderr)
                except BaseException:
                    if process.poll() is None:
                        process.kill()
                        process.communicate()
                    raise
            finally:
                release.set()
                server.shutdown()
                thread.join()
        self.assertTrue(dispatched.is_set())
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("deadline", run.stderr)
        self.assertLess(elapsed, 2.5)

    def test_panel_control_envelope(self):
        requests = []
        class Control(socketserver.StreamRequestHandler):
            def handle(self):
                while line := self.rfile.readline():
                    req = json.loads(line)
                    requests.append(req)
                    result = {"bodies": []} if req["method"] == "document.inspect" else {"created": "Box"}
                    self.wfile.write((json.dumps({"id": req["id"], "ok": True, "result": result}) + "\n").encode())
        with socketserver.TCPServer(("127.0.0.1", 0), Control) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            self.env["SOLIDCRAFT_CONTROL_TOKEN"] = "a" * 64
            try:
                output = self.host("--port", str(server.server_address[1]), "panel", "hello-panel",
                                   input='{"id":"make-box","inputs":{"size":12}}\nquit\n')
            finally:
                server.shutdown()
                thread.join()
        runs = [r for r in requests if r["method"] == "engine.execute"]
        self.assertEqual(len(runs), 1, requests)
        self.assertEqual(runs[0]["params"], {"command": "solid.box", "params": {"length": 12.0, "width": 12.0, "height": 12.0}, "allow_paths": []})
        self.assertIn("1 boxes", output)

if __name__ == "__main__":
    unittest.main()
