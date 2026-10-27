#!/usr/bin/env python3
"""Reference Print component against the real companion's simulated backend."""
import http.client
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time
import unittest
from urllib.parse import quote
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = Path(os.getenv("CARGO_TARGET_DIR", str(ROOT / "target")))
HOST = os.getenv("DUTCHMAN_PLUGINS_BIN", str(TARGET / "release/dutchman-plugins"))
PRINT = os.getenv("DUTCHMAN_PRINT_BIN", str(TARGET / "release/dutchman-print"))


def write_mesh(path):
    """A synthetic one-millimetre tetrahedron, with no external design files."""
    vertices = ''.join(f'<vertex x="{x}" y="{y}" z="{z}"/>'
                       for x, y, z in ((0, 0, 0), (1, 0, 0), (0, 1, 0), (0, 0, 1)))
    triangles = ''.join(f'<triangle v1="{a}" v2="{b}" v3="{c}"/>'
                        for a, b, c in ((0, 2, 1), (0, 1, 3), (1, 2, 3), (2, 0, 3)))
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("[Content_Types].xml", '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>')
        archive.writestr("_rels/.rels", '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>')
        archive.writestr("3D/3dmodel.model", '<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices>' + vertices + '</vertices><triangles>' + triangles + '</triangles></mesh></object></resources><build><item objectid="1"/></build></model>')


class PrintEndToEnd(unittest.TestCase):
    def setUp(self):
        # Keep all scratch files under the supplied build scratch directory.
        scratch = Path(os.getenv("TMPDIR", str(ROOT / ".build-tmp"))).resolve()
        self.assertNotEqual(scratch, Path("/tmp"))
        self.assertNotIn(Path("/tmp"), scratch.parents)
        scratch.mkdir(parents=True, exist_ok=True)
        temp = tempfile.TemporaryDirectory(prefix="print-e2e-", dir=scratch)
        self.addCleanup(temp.cleanup)
        self.directory = Path(temp.name)
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith("DUTCHMAN_PRINT_")}
        self.env.update(DUTCHMAN_PLUGINS_DATA=str(self.directory / "host"),
                        DUTCHMAN_PLUGINS_TEST="1", TMPDIR=str(self.directory))
        self.mesh = self.directory / "selection.3mf"
        write_mesh(self.mesh)
        config = self.directory / "config.toml"
        config.write_text('[backend]\nkind = "simulated"\n\n[[printers]]\n'
                          'id = "bench"\nname = "Workbench demo"\nmodel = "Simulated"\n')
        config.chmod(0o600)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            self.port = listener.getsockname()[1]
        self.log = (self.directory / "server.log").open("w+")
        self.addCleanup(self.log.close)
        self.server = subprocess.Popen(
            [PRINT, "--config", str(config), "serve", "--listen",
             f"127.0.0.1:{self.port}", "--data-dir", str(self.directory / "jobs")],
            cwd=ROOT, env=self.env, stdin=subprocess.DEVNULL,
            stdout=self.log, stderr=subprocess.STDOUT)
        self.addCleanup(self.stop_server)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.server.poll() is not None:
                self.fail("Companion exited during startup:\n" + self.server_log())
            try:
                status, health = self.http("GET", "/healthz")
                if status == 200:
                    self.assertEqual(health, {"service": "dutchman-print", "api": 1})
                    break
            except (OSError, http.client.HTTPException):
                pass
            time.sleep(0.05)
        else:
            self.fail("Companion did not become ready:\n" + self.server_log())
        self.args = [HOST, "--yes", "--local",
                     f"dutchman-print=http://127.0.0.1:{self.port}",
                     "--selection-3mf", str(self.mesh)]

    def server_log(self):
        self.log.flush()
        self.log.seek(0)
        return self.log.read()

    def stop_server(self):
        if self.server.poll() is None:
            self.server.terminate()
            try:
                self.server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.server.kill()
                self.server.wait(timeout=10)

    def http(self, method, path, data=None, token=None):
        client = http.client.HTTPConnection("127.0.0.1", self.port, timeout=2)
        try:
            headers = {"Content-Type": "application/json"}
            if token is not None:
                headers["Plugin-Confirmation"] = token
            client.request(method, path, None if data is None else json.dumps(data), headers)
            response = client.getresponse()
            body = response.read()
            return response.status, json.loads(body) if body else None
        finally:
            client.close()

    def get(self, path):
        status, value = self.http("GET", path)
        self.assertEqual(status, 200, value)
        return value

    def host(self, *args, confirm=False, input="", ok=True):
        run = subprocess.run(self.args + (["--confirm-physical"] if confirm else []) + list(args),
                             cwd=ROOT, env=self.env, input=input, text=True,
                             capture_output=True, timeout=180)
        output = run.stdout + run.stderr
        if ok:
            self.assertEqual(run.returncode, 0, output + self.server_log())
        else:
            self.assertNotEqual(run.returncode, 0, output)
        return run

    def entry(self, action, params=None, **kwargs):
        return self.host("run", "print", action, json.dumps(params or {}), **kwargs)

    def value(self, run):
        return json.loads(run.stdout.strip().splitlines()[-1])

    def review(self, action, job=None):
        params = {"action": action, "printer": "bench"}
        if job is not None:
            params["job"] = job
        challenge = self.value(self.entry("review", params))
        self.assertEqual(challenge["action"], "start" if action == "print" else action)
        self.assertEqual(challenge["printer"], "bench")
        self.assertEqual(challenge["job"], job)
        self.assertTrue(challenge["id"])
        self.assertTrue(challenge["summary"])
        self.assertGreater(challenge["expires_at"], time.time())
        self.assertIsInstance(challenge["observed"], dict)
        params.pop("action")
        params.update(confirmation=challenge["id"], summary=challenge["summary"])
        return challenge, params

    def refused(self, path, data, expected, token=None):
        status, value = self.http("POST", path, data, token)
        self.assertEqual(status, expected, value)
        self.assertTrue(value["error"], value)

    def act(self, action, params):
        run = self.entry(action, params, confirm=True)
        # The host presents the verbatim server sentence in its frozen parameters.
        self.assertIn(json.dumps(params["summary"]), run.stderr)
        return self.value(run)

    def test_prepare_clear_print_and_printer_stop(self):
        fleet = self.value(self.entry("status"))
        self.assertEqual(len(fleet), 1)
        self.assertEqual(fleet[0]["id"], "bench")
        self.assertEqual(fleet[0]["state"], "idle")
        self.assertTrue(fleet[0]["needs_clearing"])
        panel = self.host("panel", "print", input='{"id":"status","inputs":{"printer":0}}\nquit\n')
        self.assertIn("Workbench demo", panel.stdout)
        self.assertIn("Bed is clear", panel.stdout)
        job = self.value(self.entry("slice"))
        self.assertEqual(job["printer"], "bench")
        self.assertEqual(job["state"], "ready")
        size = self.mesh.stat().st_size
        self.assertEqual(job["estimate"]["seconds"], 60 + size // 1024)
        self.assertAlmostEqual(job["estimate"]["filament_g"], size / 1000)
        job_path = "/api/v1/jobs/" + quote(job["id"], safe="")
        self.assertEqual(self.get(job_path)["state"], "ready")
        self.assertEqual(self.get("/api/v1/printers")[0]["state"], "idle")

        # Neither host approval alone nor a bare API request authorizes movement.
        missing = {"printer": "bench", "job": job["id"]}
        run = self.entry("print", missing, confirm=True, ok=False)
        self.assertIn("fresh confirmation challenge", run.stderr)
        self.refused(job_path + "/start", {}, 403, secrets.token_hex(32))
        self.assertEqual(self.get(job_path)["state"], "ready")

        run = self.entry("clear", {"printer": "bench"}, confirm=True, ok=False)
        self.assertIn("fresh confirmation challenge", run.stderr)
        clear, clear_params = self.review("clear")
        self.assertTrue(clear["observed"]["needs_clearing"])
        self.refused("/api/v1/printers/bench/clear",
                     {"confirmation": clear["id"]}, 403)
        self.refused(job_path + "/start", {"confirmation": clear["id"]},
                     409, secrets.token_hex(32))
        self.assertEqual(self.get(job_path)["state"], "ready")
        self.assertIsNone(self.act("clear", clear_params))
        self.assertFalse(self.get("/api/v1/printers")[0]["needs_clearing"])
        self.refused("/api/v1/printers/bench/clear",
                     {"confirmation": clear["id"]}, 409, secrets.token_hex(32))

        start, start_params = self.review("print", job["id"])
        self.assertNotEqual(start["id"], clear["id"])
        self.assertFalse(start["observed"]["needs_clearing"])
        self.assertEqual(start["observed"]["state"], "idle")
        printing = self.act("print", start_params)
        self.assertEqual(printing["id"], job["id"])
        self.assertEqual(printing["state"], "printing")
        self.assertEqual(self.get(job_path)["state"], "printing")
        fleet = self.value(self.entry("status"))
        self.assertEqual(fleet[0]["state"], "printing")
        self.assertEqual(fleet[0]["job_id"], job["id"])
        self.assertTrue(fleet[0]["needs_clearing"])

        # The component refuses a second Print; the real server also rejects replay
        # even with a fresh host-style header. No automatic retry is authorized.
        run = self.entry("print", start_params, confirm=True, ok=False)
        self.assertIn("fresh confirmation challenge", run.stderr)
        self.refused(job_path + "/start", {"confirmation": start["id"]},
                     409, secrets.token_hex(32))
        self.assertEqual(self.get(job_path)["state"], "printing")
        run = self.entry("stop", {"printer": "bench"}, confirm=True, ok=False)
        self.assertIn("fresh confirmation challenge", run.stderr)
        self.refused("/api/v1/printers/bench/stop", {}, 403, secrets.token_hex(32))

        stop, stop_params = self.review("stop")
        self.assertNotIn(stop["id"], (start["id"], clear["id"]))
        self.assertIsNone(stop["job"])
        self.assertEqual(stop["observed"]["state"], "printing")
        self.assertEqual(stop["observed"]["job_id"], job["id"])
        stopped = self.act("stop", stop_params)
        self.assertEqual(stopped["id"], "bench")
        self.assertEqual(stopped["state"], "idle")
        self.assertTrue(stopped["needs_clearing"])
        self.assertEqual(self.get(job_path)["state"], "cancelled")
        fleet = self.value(self.entry("status"))
        self.assertEqual(fleet[0]["state"], "idle")
        self.assertFalse(fleet[0].get("job_id"))
        self.assertTrue(fleet[0]["needs_clearing"])
        self.refused("/api/v1/printers/bench/stop", {"confirmation": stop["id"]},
                     409, secrets.token_hex(32))
        self.assertEqual(self.get(job_path)["state"], "cancelled")


if __name__ == "__main__":
    unittest.main()
