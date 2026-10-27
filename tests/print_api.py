"""Print component integration against a loopback Print API v1 fake."""
import http.client
import http.server
import io
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import time
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
QUEUE_TARGET = Path.home() / ".cache/buildq/target/plugins" / f"{ROOT.name}-{hashlib.sha1(str(ROOT).encode()).hexdigest()[:8]}"
TARGET = Path(os.getenv("CARGO_TARGET_DIR", str(ROOT / "target" if (ROOT / "target").exists() else QUEUE_TARGET)))
BIN = os.getenv("DUTCHMAN_PLUGINS_BIN", str(TARGET / "release/dutchman-plugins"))


class PrintSmoke(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.env = dict(os.environ, DUTCHMAN_PLUGINS_DATA=self.temp.name,
                        DUTCHMAN_PLUGINS_TEST="1")
        self.mesh = Path(self.temp.name) / "selection.3mf"
        with zipfile.ZipFile(self.mesh, "w") as archive:
            archive.writestr("[Content_Types].xml", '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>')
            archive.writestr("_rels/.rels", '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>')
            archive.writestr("3D/3dmodel.model", '<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/></vertices><triangles><triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/><triangle v1="1" v2="2" v3="3"/><triangle v1="2" v2="0" v3="3"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>')
        self.requests = []
        self.tokens = set()
        self.state = "idle"
        self.needs_clearing = True
        self.error = None
        case = self

        class API(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, status, value):
                body = json.dumps(value).encode() if status != 204 else b""
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def job(self, state):
                return {"id": "job/one", "printer": "bench", "name": "Selection",
                        "state": state, "estimate": {"seconds": 120, "filament_g": 4.5},
                        "progress": 0.25, "seconds_left": 90}

            def do_GET(self):
                case.requests.append((time.monotonic(), "GET", self.path, {k.lower(): v for k, v in self.headers.items()}, b""))
                if case.error:
                    return self.reply(503, {"error": case.error})
                if self.path == "/api/v1/printers":
                    return self.reply(200, [{"id": "bench", "name": "Workbench", "model": "Fake",
                                             "state": case.state, "progress": 0.25,
                                             "seconds_left": 90, "needs_clearing": case.needs_clearing,
                                             "job_id": "job/one"}])
                self.reply(404, {"error": "Job expired."})

            def do_POST(self):
                body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                case.requests.append((time.monotonic(), "POST", self.path, {k.lower(): v for k, v in self.headers.items()}, body))
                if self.path.startswith("/api/v1/jobs?"):
                    if self.headers.get("Content-Type") != "model/3mf":
                        return self.reply(415, {"error": "Send model/3mf."})
                    with zipfile.ZipFile(io.BytesIO(body)) as archive:
                        if "3D/3dmodel.model" not in archive.namelist():
                            return self.reply(400, {"error": "Invalid 3MF."})
                    return self.reply(201, self.job("ready"))
                values = self.headers.get_all("Plugin-Confirmation", [])
                if len(values) != 1 or not re.fullmatch("[0-9a-f]{64}", values[0]) or values[0] in case.tokens:
                    return self.reply(403, {"error": "Confirmation required."})
                case.tokens.add(values[0])
                if self.path.endswith("/clear"):
                    case.needs_clearing = False
                    return self.reply(204, None)
                if self.path.endswith("/start"):
                    if case.needs_clearing:
                        return self.reply(409, {"error": "Check Workbench bed."})
                    case.state = "printing"
                    case.needs_clearing = True
                elif self.path.endswith("/pause"):
                    case.state = "paused"
                elif self.path.endswith("/resume"):
                    case.state = "printing"
                elif self.path.endswith("/cancel"):
                    case.state = "idle"
                else:
                    return self.reply(404, {"error": "Unknown route."})
                value = self.job("cancelled" if self.path.endswith("/cancel") else case.state)
                value["echo"] = values[0]
                self.reply(200, value)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), API)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.stop_server)
        self.args = [BIN, "--yes", "--local",
                     f"dutchman-print=http://127.0.0.1:{self.server.server_port}",
                     "--selection-3mf", str(self.mesh)]

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def run_host(self, *args, confirm=False, input="", ok=True):
        run = subprocess.run(self.args + (["--confirm-physical"] if confirm else []) + list(args),
                             env=self.env, cwd=ROOT, input=input, text=True,
                             capture_output=True, timeout=180)
        if ok:
            self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        else:
            self.assertNotEqual(run.returncode, 0, run.stdout + run.stderr)
        return run

    def test_print_prepare_confirm_controls_and_widget(self):
        run = self.run_host("run", "print", "slice")
        self.assertIn('"seconds":120', run.stdout)
        request = self.requests[-1]
        self.assertEqual(request[2], "/api/v1/jobs?printer=bench")
        self.assertEqual(request[3]["content-type"], "model/3mf")
        self.assertEqual(request[4], self.mesh.read_bytes())
        self.assertNotIn("plugin-confirmation", request[3])
        count = len(self.requests)
        run = self.run_host("run", "print", "print", ok=False)
        self.assertIn("confirmation", run.stderr)
        self.assertEqual(len(self.requests), count)
        self.run_host("run", "print", "clear", '{"printer":"bench"}', confirm=True)
        run = self.run_host("run", "print", "print", '{"printer":"bench","job":"job/one"}', confirm=True)
        self.assertIn('"echo":"[secret]"', run.stdout)
        self.assertTrue(self.requests[-1][2].endswith("/job%2Fone/start"))
        run = self.run_host("panel", "print", input="quit\n")
        self.assertIn("Workbench", run.stdout)
        self.assertIn("Widget:", run.stdout)
        self.assertIn("0.25", run.stdout)
        self.assertIn("90 s left", run.stdout)
        self.assertNotIn("plugin-confirmation", self.requests[-1][3])
        for action in ("pause", "resume", "cancel", "clear"):
            self.run_host("run", "print", action, '{"printer":"bench","job":"job/one"}', confirm=True)
        self.assertEqual(len(self.tokens), 6)
        self.assertNotIn(next(iter(self.tokens)), run.stdout + run.stderr)
        # The companion consumes tokens across routes, including failed attempts.
        client = http.client.HTTPConnection("127.0.0.1", self.server.server_port)
        try:
            client.request("POST", "/api/v1/jobs/job%2Fone/start")
            response = client.getresponse()
            self.assertEqual(response.status, 403)
            response.read()
            client.request("POST", "/api/v1/printers/bench/clear", headers={"Plugin-Confirmation": next(iter(self.tokens))})
            response = client.getresponse()
            self.assertEqual(response.status, 403)
            response.read()
        finally:
            client.close()

    def test_profile_overrides_are_encoded(self):
        self.run_host("run", "print", "slice", '{"printer":"bench","print_profile":"Fine & slow","filament_profile":"PLA+"}')
        self.assertEqual(self.requests[-1][2], "/api/v1/jobs?printer=bench&print_profile=Fine%20%26%20slow&filament_profile=PLA%2B")

    def test_panel_estimate_and_unconfirmed_button(self):
        run = self.run_host("panel", "print", input='{"id":"slice"}\n{"id":"print"}\nquit\n')
        self.assertIn("Estimate:", run.stdout)
        self.assertIn("120", run.stdout)
        self.assertIn("confirmation", run.stdout + run.stderr)
        self.assertFalse(any(r[2].endswith("/start") for r in self.requests))

    def test_companion_error_is_plain_text(self):
        self.error = "Printer is unavailable."
        run = self.run_host("panel", "print", input="quit\n")
        self.assertIn(self.error, run.stdout)

    def test_refresh_pacing_and_visibility(self):
        self.state = "printing"
        process = subprocess.Popen(self.args + ["panel", "print"], cwd=ROOT, env=self.env,
                                   stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.monotonic() + 120
            while not self.requests and process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.05)
            self.assertTrue(self.requests)
            # The widget remains visible after the panel closes.
            process.stdin.write("widget\n")
            process.stdin.flush()
            started = self.requests[0][0]
            while len(self.requests) < 2 and time.monotonic() < started + 8:
                time.sleep(0.05)
            self.assertEqual(len(self.requests), 2)
            self.assertGreaterEqual(self.requests[1][0] - started, 5)
            process.stdin.write("hide\n")
            process.stdin.flush()
            time.sleep(5.3)
            self.assertEqual(len(self.requests), 2)
            process.stdin.write("show\n")
            process.stdin.flush()
            time.sleep(1)
            self.assertEqual(len(self.requests), 2)
            time.sleep(4.3)
            self.assertEqual(len(self.requests), 3)
            _, stderr = process.communicate("quit\n", timeout=15)
            self.assertEqual(process.returncode, 0, stderr)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()

    def test_idle_widget_and_thirty_second_refresh(self):
        process = subprocess.Popen(self.args + ["panel", "print"], cwd=ROOT, env=self.env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.monotonic() + 120
            while not self.requests and process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.05)
            self.assertTrue(self.requests)
            started = self.requests[0][0]
            time.sleep(5.2)
            self.assertEqual(len(self.requests), 1)
            while len(self.requests) < 2 and time.monotonic() < started + 33:
                time.sleep(0.1)
            self.assertEqual(len(self.requests), 2)
            self.assertGreaterEqual(self.requests[1][0] - started, 30)
            stdout, stderr = process.communicate("quit\n", timeout=15)
            self.assertEqual(process.returncode, 0, stderr)
            self.assertIn("1 printers idle", stdout)
            self.assertIn("Bed is clear", stdout)
            self.assertIn("Attention", stdout)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()

    def test_confirmation_switch_requires_test_environment(self):
        self.env.pop("DUTCHMAN_PLUGINS_TEST")
        run = self.run_host("run", "print", "clear", '{"printer":"bench"}', confirm=True, ok=False)
        self.assertIn("for tests only", run.stderr)
        self.assertEqual(self.requests, [])

    def test_public_address_refused(self):
        self.args = [BIN, "--yes", "--local", "dutchman-print=http://8.8.8.8"]
        run = self.run_host("run", "print", "status", ok=False)
        self.assertIn("outside allowed ranges", run.stderr)
        self.assertEqual(self.requests, [])


if __name__ == "__main__":
    unittest.main()
