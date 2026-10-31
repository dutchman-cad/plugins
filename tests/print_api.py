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
from urllib.parse import urlsplit, parse_qs, unquote
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
        self.challenges = {}
        self.job_state = "ready"
        self.digest = "prepared-input"
        self.live_job_id = "job/one"
        self.live_job_name = "Selection"
        self.backend_failure = False
        self.detail = None
        self.progress = 0.25
        self.change_on_start = False
        self.challenge_seconds = 60
        self.state = "idle"
        self.needs_clearing = True
        self.controls = ["start", "pause", "resume", "cancel", "stop", "clear"]
        self.error = None
        self.extra_printers = []
        self.queue = []
        self.history = []
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
                if self.path == "/api/v1/queue":
                    return self.reply(200, case.queue)
                if self.path == "/api/v1/queue/proposals":
                    return self.reply(200, [{"id": e["id"], "printer": None if e["attempted"] or case.needs_clearing else "bench",
                                             "reason": e["reason"]} for e in case.queue])
                if self.path.startswith("/api/v1/history?"):
                    query = parse_qs(urlsplit(self.path).query)
                    return self.reply(200, [r for r in case.history if r["printer"] == query["printer"][0]][:int(query["limit"][0])])
                if self.path == "/api/v1/jobs/job%2Fone":
                    return self.reply(200, self.job(case.job_state))
                if self.path == "/api/v1/printers":
                    return self.reply(200, [{"id": "bench", "name": "Workbench", "model": "Fake",
                                             "state": case.state, "progress": case.progress,
                                             "seconds_left": 90, "needs_clearing": case.needs_clearing,
                                             "job_id": case.live_job_id, "job_name": case.live_job_name,
                                             "detail": case.detail,
                                             **({"controls": case.controls} if case.controls is not None else {})}] + case.extra_printers)
                self.reply(404, {"error": "Job expired."})

            def do_DELETE(self):
                case.requests.append((time.monotonic(), "DELETE", self.path, {}, b""))
                entry_id = unquote(self.path.rsplit("/", 1)[-1])
                case.queue[:] = [e for e in case.queue if e["id"] != entry_id]
                self.reply(204, None)

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
                path = urlsplit(self.path).path
                try:
                    data = json.loads(body) if body else {}
                except (ValueError, UnicodeDecodeError):
                    return self.reply(400, {"error": "Send a JSON object."})
                if not isinstance(data, dict):
                    return self.reply(400, {"error": "Send a JSON object."})
                queue_entry = None
                if path == "/api/v1/queue":
                    entry = dict(id=f"entry/{len(case.queue) + 1}", job=data["job"], target=data["target"],
                                 profile=None, kind=None, reason=None, prepared=None, attempted=False)
                    case.queue.append(entry)
                    return self.reply(201, entry)
                if path.startswith("/api/v1/queue/"):
                    entry_id = unquote(path.split("/")[-2])
                    queue_entry = next(e for e in case.queue if e["id"] == entry_id)
                    operation = path.rsplit("/", 1)[-1]
                    if operation == "move":
                        case.queue.remove(queue_entry)
                        case.queue.insert(data["position"], queue_entry)
                        return self.reply(200, case.queue)
                    if operation == "confirmations":
                        queue_entry["prepared"] = "job/one"
                        path = "/api/v1/confirmations"
                        data = dict(action="start", printer=data["printer"], job="job/one")
                    elif operation == "dispatch":
                        if queue_entry["attempted"]:
                            return self.reply(409, {"error": "Dispatch was already attempted."})
                        path = "/api/v1/jobs/job%2Fone/start"
                def observation():
                    return {"state": case.state, "detail": case.detail, "job_id": case.live_job_id,
                            "job_name": case.live_job_name, "progress": int(case.progress * 100) / 100,
                            "needs_clearing": case.needs_clearing}
                if path == "/api/v1/confirmations":
                    action, printer, job = data.get("action"), data.get("printer"), data.get("job")
                    if action not in ("start", "cancel", "pause", "resume", "clear", "stop") or not isinstance(printer, str):
                        return self.reply(400, {"error": "Invalid action or printer."})
                    if printer != "bench":
                        return self.reply(404, {"error": "Printer not found."})
                    if (action in ("start", "cancel") and not isinstance(job, str)) or (action not in ("start", "cancel") and "job" in data):
                        return self.reply(400, {"error": "Invalid job binding."})
                    if action in ("start", "cancel") and job != "job/one":
                        return self.reply(404, {"error": "Job not found."})
                    if case.controls is not None and action not in case.controls:
                        return self.reply(409, {"error": "Action not configured for printer."})
                    compatible = {"start": case.state == "idle" and not case.needs_clearing and case.job_state == "ready",
                                  "clear": case.state == "idle", "pause": case.state == "printing",
                                  "resume": case.state == "paused", "cancel": case.job_state in ("ready", "printing", "paused"),
                                  "stop": case.state in ("printing", "paused", "attention") and bool(case.live_job_id)}
                    if not compatible[action]:
                        return self.reply(409, {"error": "Action incompatible with printer."})
                    challenge = {"id": str(len(case.challenges) + 1), "action": action, "printer": printer,
                                 "job": job, "summary": f"{action.title()} Selection on Workbench ({case.state}, bed {'not clear' if case.needs_clearing else 'clear'})?",
                                 "observed": observation(), "expires_at": int(time.time()) + case.challenge_seconds}
                    case.challenges[challenge["id"]] = dict(challenge, used=False, digest=case.digest, job_state=case.job_state)
                    return self.reply(201, challenge)
                if not re.fullmatch(r"/api/v1/(jobs/[^/]+/(start|cancel)|printers/[^/]+/(pause|resume|clear|stop))", path):
                    return self.reply(404, {"error": "Unknown route."})
                action = path.rsplit("/", 1)[-1]
                printer = "bench" if path.startswith("/api/v1/jobs/") else unquote(path.split("/")[-2])
                job = unquote(path.split("/")[-2]) if action in ("start", "cancel") else None
                if case.controls is not None and action not in case.controls:
                    return self.reply(409, {"error": "Action not configured for printer."})
                values = self.headers.get_all("Plugin-Confirmation", [])
                challenge_id = data.get("confirmation") or parse_qs(urlsplit(self.path).query).get("confirmation", [None])[0]
                if len(values) != 1 or not re.fullmatch("[0-9a-f]{64}", values[0]) or not isinstance(challenge_id, str) or not challenge_id:
                    return self.reply(403, {"error": "Confirmation required."})
                challenge = case.challenges.get(challenge_id)
                if values[0] in case.tokens or not challenge or challenge["used"] or challenge["expires_at"] <= time.time():
                    return self.reply(409, {"error": "Invalid, expired or consumed confirmation; confirm again."})
                if (challenge["action"], challenge["printer"], challenge["job"], challenge["digest"]) != (action, printer, job, case.digest):
                    return self.reply(409, {"error": "Confirmation binding changed; confirm again."})
                if action == "start" and case.change_on_start:
                    case.detail = "Changed telemetry"
                    case.change_on_start = False
                if challenge["observed"] != observation() or challenge["job_state"] != case.job_state:
                    return self.reply(409, {"error": "The printer changed since you confirmed; confirm again."})
                challenge["used"] = True
                case.tokens.add(values[0])
                if queue_entry is not None:
                    queue_entry["attempted"] = True
                    if case.backend_failure:
                        queue_entry["reason"] = "Backend unavailable."
                    else:
                        case.queue.remove(queue_entry)
                if case.backend_failure:
                    return self.reply(503, {"error": "Backend unavailable."})
                if action == "clear":
                    case.needs_clearing = False
                    return self.reply(204, None)
                if action == "start":
                    case.state = "printing"
                    case.needs_clearing = True
                elif action == "pause":
                    case.state = "paused"
                elif action == "resume":
                    case.state = "printing"
                elif action == "cancel":
                    case.state = "idle"
                elif action == "stop":
                    case.state = "idle"
                    case.job_state = "cancelled"
                    return self.reply(200, {"id": "bench", "name": "Workbench", "model": "Fake", "state": "idle",
                                            "needs_clearing": True, "echo": values[0],
                                            **({"controls": case.controls} if case.controls is not None else {})})
                else:
                    return self.reply(404, {"error": "Unknown route."})
                case.job_state = "cancelled" if action == "cancel" else case.state
                value = self.job(case.job_state)
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
        self.act("clear")
        run = self.run_host("run", "print", "print", '{"printer":"bench","job":"job/one"}', confirm=True, ok=False)
        self.assertIn("fresh confirmation challenge", run.stderr)
        self.assertFalse(any(r[2].endswith("/start") for r in self.requests))
        run = self.act("print")
        self.assertIn('"echo":"[secret]"', run.stdout)
        self.assertTrue(self.requests[-1][2].endswith("/job%2Fone/start"))
        run = self.run_host("panel", "print", input="quit\n")
        self.assertIn("Workbench", run.stdout)
        self.assertIn("Widget:", run.stdout)
        self.assertIn("0.25", run.stdout)
        self.assertIn("90 s left", run.stdout)
        for action in ("pause", "resume", "cancel", "clear"):
            self.act(action)
        self.assertEqual(len(self.tokens), 6)

    def challenge(self, action):
        params = {"action": action, "printer": "bench"}
        if action in ("print", "cancel"):
            params["job"] = "job/one"
        run = self.run_host("run", "print", "review", json.dumps(params))
        value = json.loads(run.stdout.strip().splitlines()[-1])
        return dict(params, confirmation=value["id"], summary=value["summary"])

    def act(self, action):
        return self.run_host("run", "print", action, json.dumps(self.challenge(action)), confirm=True)

    def post(self, path, data=None, token="a" * 64):
        client = http.client.HTTPConnection("127.0.0.1", self.server.server_port)
        try:
            headers = {"Content-Type": "application/json"}
            if token is not None:
                headers["Plugin-Confirmation"] = token
            client.request("POST", path, json.dumps(data or {}), headers)
            response = client.getresponse()
            return response.status, response.read()
        finally:
            client.close()

    def queue_command(self, action, params=None, **kwargs):
        return self.run_host("run", "print", action, json.dumps(params or {}), **kwargs)

    def queue_review(self, entry_id):
        run = self.queue_command("review", dict(action="dispatch", queue=entry_id, printer="bench"))
        challenge = json.loads(run.stdout.strip().splitlines()[-1])
        return dict(queue=entry_id, printer=challenge["printer"], job=challenge["job"],
                    confirmation=challenge["id"], summary=challenge["summary"])

    def test_queue_management_history_and_unconfirmed_dispatch(self):
        self.queue_command("slice")
        self.queue_command("queue-add", {"target": "bench"})
        self.queue_command("queue-add", {"target": "any"})
        self.assertEqual([e["target"] for e in self.queue], ["bench", "any"])
        self.queue_command("queue-move", {"queue": "entry/2", "position": 0})
        self.assertEqual(self.queue[0]["id"], "entry/2")
        self.assertIn('entry/2', self.queue_command("queue").stdout)
        self.queue_command("queue-remove", {"queue": "entry/1"})
        self.assertEqual(len(self.queue), 1)
        self.needs_clearing = False
        self.history = [dict(printer="bench", job_name=f"Part {n}", outcome="done",
                             ended_at="2026-10-30T12:00:00Z", duration_seconds=120) for n in range(6)]
        run = self.run_host("panel", "print", input='{"id":"dispatch:entry/2"}\nquit\n')
        for label in ("1. Selection", "target: any", "proposed printer: bench", "Recent prints",
                      "Part 0", "Part 4", "done", "2026-10-30T12:00:00Z", "120 s"):
            self.assertIn(label, run.stdout)
        self.assertNotIn("Part 5", run.stdout)
        self.assertFalse(any(r[2].endswith("/dispatch") for r in self.requests))
        self.assertFalse(any("plugin-confirmation" in r[3] for r in self.requests))

    def test_panel_queue_actions_and_paging(self):
        self.run_host("panel", "print", input='{"id":"slice"}\n{"id":"queue-this"}\nquit\n')
        self.assertEqual(self.queue[0]["target"], "bench")
        self.run_host("panel", "print", input='{"id":"remove:entry/1"}\n{"id":"queue-any"}\nquit\n')
        self.assertEqual(self.queue[0]["target"], "any")
        self.assertTrue(any(r[1:3] == ("DELETE", "/api/v1/queue/entry%2F1") for r in self.requests))
        self.queue_command("queue-add", {"target": "bench"})
        self.run_host("panel", "print", input='{"id":"up:entry/2"}\nquit\n')
        self.assertEqual(self.queue[0]["target"], "bench")
        self.queue = [dict(id=f"entry/{n}", job="job/one", target="any", reason=None,
                           prepared=None, attempted=False) for n in range(1, 22)]
        run = self.run_host("panel", "print", input='{"id":"queue-next"}\nquit\n')
        self.assertIn("Queue page 1 of 2", run.stdout)
        self.assertIn("Queue page 2 of 2", run.stdout)
        self.assertIn('id: "remove:entry/21"', run.stdout)
        self.assertFalse(any("plugin-confirmation" in r[3] for r in self.requests))

    def test_dispatch_changed_observation_needs_another_approval(self):
        self.needs_clearing = False
        self.queue_command("slice")
        self.queue_command("queue-add")
        stale = self.queue_review("entry/1")
        self.change_on_start = True
        self.queue_command("dispatch", stale, confirm=True, ok=False)
        self.queue_command("dispatch", stale, confirm=True, ok=False)
        self.assertEqual(len([r for r in self.requests if r[2].endswith("/dispatch")]), 1)
        fresh = self.queue_review("entry/1")
        self.assertNotEqual(fresh["confirmation"], stale["confirmation"])
        # Reviewing never starts it; a separate confirmation is still needed.
        self.assertEqual(self.state, "idle")
        self.queue_command("dispatch", fresh, ok=False)
        self.assertEqual(self.state, "idle")
        self.queue_command("dispatch", fresh, confirm=True)
        self.assertEqual(self.state, "printing")
        self.assertEqual(self.queue, [])
        self.queue_command("dispatch", fresh, confirm=True, ok=False)
        dispatches = [r for r in self.requests if r[2].endswith("/dispatch")]
        self.assertEqual(len(dispatches), 2)
        self.assertNotEqual(dispatches[0][3]["plugin-confirmation"], dispatches[1][3]["plugin-confirmation"])

    def test_dispatch_fresh_binding_and_single_attempt(self):
        self.needs_clearing = False
        self.queue_command("slice")
        self.queue_command("queue-add")
        self.queue_command("dispatch", dict(queue="entry/1", printer="bench"), confirm=True, ok=False)
        self.assertFalse(any(r[2].endswith("/dispatch") for r in self.requests))
        params = self.queue_review("entry/1")
        for changed in (dict(queue="another"), dict(job="another"), dict(printer="another"), dict(summary="changed")):
            self.queue_command("dispatch", dict(params, **changed), confirm=True, ok=False)
        self.assertFalse(any(r[2].endswith("/dispatch") for r in self.requests))
        self.backend_failure = True
        self.queue_command("dispatch", params, confirm=True, ok=False)
        self.queue_command("dispatch", params, confirm=True, ok=False)
        dispatches = [r for r in self.requests if r[2].endswith("/dispatch")]
        self.assertEqual(len(dispatches), 1)
        self.assertRegex(dispatches[0][3]["plugin-confirmation"], "^[0-9a-f]{64}$")
        self.assertTrue(self.queue[0]["attempted"])
        run = self.run_host("panel", "print", input="quit\n")
        self.assertIn("Backend unavailable.", run.stdout)
        self.assertIn("reserved until removed", run.stdout)
        self.assertNotIn('id: "dispatch:entry/1"', run.stdout)

    def test_challenge_binding_expiry_replay_and_header(self):
        self.run_host("run", "print", "slice")
        clear = self.challenge("clear")
        self.assertEqual(self.post("/api/v1/jobs/job%2Fone/start", clear)[0], 409)
        self.assertEqual(self.post("/api/v1/printers/bench/clear", clear, token=None)[0], 403)
        self.assertEqual(self.post("/api/v1/printers/bench/clear", clear)[0], 204)
        self.assertEqual(self.post("/api/v1/printers/bench/clear", clear, token="b" * 64)[0], 409)
        self.assertEqual(self.post("/api/v1/jobs/job%2Fone/start")[0], 403)
        start = self.challenge("print")
        self.challenges[start["confirmation"]]["expires_at"] = 0
        self.assertEqual(self.post("/api/v1/jobs/job%2Fone/start", start, token="c" * 64)[0], 409)
        start = self.challenge("print")
        self.assertEqual(self.post("/api/v1/jobs/job%2Fone/start", start)[0], 409)  # reused header
        self.digest = "changed-input"
        self.assertEqual(self.post("/api/v1/jobs/job%2Fone/start", start, token="d" * 64)[0], 409)

    def test_header_format_and_duplicate_headers(self):
        status, body = self.post("/api/v1/confirmations", {"action": "clear", "printer": "bench"}, token=None)
        self.assertEqual(status, 201)
        data = {"confirmation": json.loads(body)["id"]}
        for token in (None, "", "A" * 64, "a" * 63, "a" * 65):
            self.assertEqual(self.post("/api/v1/printers/bench/clear", data, token)[0], 403)
        client = http.client.HTTPConnection("127.0.0.1", self.server.server_port)
        try:
            body = json.dumps(data).encode()
            client.putrequest("POST", "/api/v1/printers/bench/clear")
            client.putheader("Plugin-Confirmation", "a" * 64)
            client.putheader("Plugin-Confirmation", "b" * 64)
            client.putheader("Content-Length", str(len(body)))
            client.endheaders(body)
            response = client.getresponse()
            self.assertEqual(response.status, 403)
            response.read()
        finally:
            client.close()
        self.assertFalse(self.tokens)

    def test_observation_binding_and_failed_attempt_consumption(self):
        for key, value in (("state", "paused"), ("detail", "Changed"), ("live_job_id", "other"),
                           ("live_job_name", "Other"), ("progress", 0.26),
                           ("needs_clearing", False), ("job_state", "failed")):
            with self.subTest(field=key):
                challenge = self.challenge("clear")
                previous = getattr(self, key)
                setattr(self, key, value)
                status, body = self.post("/api/v1/printers/bench/clear", challenge)
                self.assertEqual(status, 409)
                self.assertIn(b"confirm again", body)
                setattr(self, key, previous)
        challenge = self.challenge("clear")
        self.backend_failure = True
        self.assertEqual(self.post("/api/v1/printers/bench/clear", challenge)[0], 503)
        self.backend_failure = False
        self.assertEqual(self.post("/api/v1/printers/bench/clear", challenge, token="b" * 64)[0], 409)
        challenge = self.challenge("clear")
        self.assertEqual(self.post("/api/v1/printers/bench/clear", challenge)[0], 409)
        # Empty-body query transport is equivalent to JSON; progress binds to whole percent.
        self.progress = 0.259
        self.assertEqual(self.post("/api/v1/printers/bench/clear?confirmation=" + challenge["confirmation"], token="c" * 64)[0], 204)

    def test_changed_printer_panel_requires_another_press(self):
        self.needs_clearing = False
        self.change_on_start = True
        run = self.run_host("panel", "print", input='{"id":"slice"}\n{"id":"print"}\nquit\n', confirm=True)
        self.assertIn("The printer changed since you confirmed; confirm again.", run.stdout)
        starts = [r for r in self.requests if r[2].endswith("/start")]
        self.assertEqual(len(starts), 1)
        reviews = [r for r in self.requests if r[2] == "/api/v1/confirmations" and json.loads(r[4])["action"] == "start"]
        self.assertEqual(len(reviews), 2)
        self.assertEqual(self.state, "idle")
        self.assertEqual(json.loads(reviews[-1][4])["printer"], "bench")
        sent = json.loads(starts[0][4])["confirmation"]
        self.assertIn(self.challenges[sent]["summary"], run.stdout)

    def test_panel_refreshes_expired_challenges(self):
        self.challenge_seconds = 2
        process = subprocess.Popen(self.args + ["panel", "print"], cwd=ROOT, env=self.env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True)
        try:
            deadline = time.monotonic() + 120
            while not self.challenges and process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.05)
            self.assertTrue(self.challenges)
            time.sleep(2.1)
            stdout, stderr = process.communicate('{"id":"status"}\nquit\n', timeout=30)
            self.assertEqual(process.returncode, 0, stderr)
            self.assertEqual(len(self.challenges), 2)
            self.assertIn(self.challenges["2"]["summary"], stdout)
            self.assertFalse(any(r[2].endswith("/clear") for r in self.requests))
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()

    def test_panel_clear_and_print_have_separate_approvals(self):
        run = self.run_host("panel", "print", input='{"id":"slice"}\n{"id":"clear"}\n{"id":"print"}\nquit\n', confirm=True)
        actions = [r for r in self.requests if r[2].endswith(("/clear", "/start"))]
        self.assertEqual(len(actions), 2)
        ids = [json.loads(r[4])["confirmation"] for r in actions]
        self.assertNotEqual(*ids)
        self.assertEqual([self.challenges[i]["action"] for i in ids], ["clear", "start"])
        self.assertTrue(all(self.challenges[i]["summary"] in run.stdout for i in ids))
        for challenge_id in ids:
            self.assertIn(json.dumps(self.challenges[challenge_id]["summary"]), run.stderr)
        self.assertNotEqual(actions[0][3]["plugin-confirmation"], actions[1][3]["plugin-confirmation"])
        self.assertTrue(all("plugin-confirmation" not in r[3] for r in self.requests if r[2] == "/api/v1/confirmations"))

    def test_attention_shows_reason_and_allows_only_stop(self):
        self.state = "attention"
        self.detail = "Filament runout"
        self.needs_clearing = False
        self.job_state = "printing"
        run = self.run_host("panel", "print", input='{"id":"pause"}\n{"id":"resume"}\n{"id":"stop"}\nquit\n')
        self.assertIn("Workbench: attention", run.stdout)
        self.assertIn("Filament runout", run.stdout)
        self.assertIn("Attention", run.stdout)
        reviewed = [json.loads(r[4])["action"] for r in self.requests if r[2] == "/api/v1/confirmations"]
        self.assertIn("stop", reviewed)
        issued = [c["action"] for c in self.challenges.values()]
        self.assertEqual(issued, [a for a in issued if a == "stop"])
        self.assertFalse(any(r[2].endswith(("/pause", "/resume", "/start")) for r in self.requests))

    def test_stop_ends_an_external_live_job_with_its_own_challenge(self):
        self.state = "printing"
        self.needs_clearing = False
        self.live_job_name = "External cube"
        run = self.run_host("panel", "print", input='{"id":"stop"}\n{"id":"stop"}\nquit\n', confirm=True)
        stops = [r for r in self.requests if r[2] == "/api/v1/printers/bench/stop"]
        self.assertEqual(len(stops), 1)
        challenge = self.challenges[json.loads(stops[0][4])["confirmation"]]
        self.assertEqual((challenge["action"], challenge["job"]), ("stop", None))
        self.assertIn(challenge["summary"], run.stdout)
        self.assertFalse(any(r[2].endswith("/cancel") for r in self.requests))
        self.assertEqual(self.state, "idle")

    def panel_buttons(self, output):
        return {k: v for k, v in re.findall(r'Button \{\s*id: "([^"]+)",\s*label: "[^"]+",\s*enabled: (true|false)', output) if not k.startswith("queue-")}

    def reviewed_actions(self):
        return [json.loads(r[4])["action"] for r in self.requests if r[2] == "/api/v1/confirmations"]

    def test_monitoring_only_has_no_machine_buttons_or_challenges(self):
        self.controls = []
        for state in ("idle", "printing", "paused", "attention"):
            with self.subTest(state=state):
                self.state = state
                run = self.run_host("panel", "print", input='{"id":"slice"}\nquit\n')
                self.assertEqual(self.panel_buttons(run.stdout), {"slice": "true"})
                self.assertEqual(run.stdout.count("This printer is monitoring only."), 2)
        self.assertEqual(self.reviewed_actions(), [])
        self.run_host("run", "print", "review", '{"action":"clear","printer":"bench"}', ok=False)
        self.assertEqual(self.reviewed_actions(), [])

    def test_controls_follow_the_selected_printer(self):
        self.extra_printers = [{"id": "monitor", "name": "Monitor", "state": "printing", "controls": []}]
        run = self.run_host("panel", "print", input='{"id":"status","inputs":{"printer":1}}\nquit\n')
        before, after = run.stdout.split("Print selected bodies")[1:]
        self.assertIn("clear", self.panel_buttons(before))
        self.assertEqual(self.panel_buttons(after), {"slice": "true"})
        self.assertIn("This printer is monitoring only.", after)
        self.assertEqual(self.reviewed_actions(), ["clear"])
        self.assertTrue(all(c["printer"] == "bench" for c in self.challenges.values()))

    def test_stop_only_offers_only_stop(self):
        self.controls = ["stop"]
        for state in ("idle", "printing", "paused", "attention"):
            with self.subTest(state=state):
                self.state = state
                run = self.run_host("panel", "print", input='{"id":"slice"}\nquit\n')
                self.assertEqual(self.panel_buttons(run.stdout),
                                 {"slice": "true", "stop": "false" if state == "idle" else "true"})
                self.assertNotIn("monitoring only", run.stdout)
        self.assertTrue(self.reviewed_actions())
        self.assertEqual(set(self.reviewed_actions()), {"stop"})

    def test_repetier_controls_hide_pause_resume_and_stop(self):
        self.controls = ["start", "cancel", "clear"]
        for state in ("idle", "printing", "paused", "attention"):
            with self.subTest(state=state):
                self.state = state
                run = self.run_host("panel", "print", input='{"id":"slice"}\nquit\n')
                buttons = self.panel_buttons(run.stdout)
                self.assertEqual(set(buttons), {"slice", "print", "cancel", "clear"})
                self.assertEqual(buttons["print"], "false")  # The bed still needs clearing.
                self.assertEqual(buttons["cancel"], "true" if state == "idle" else "false")
                self.assertEqual(buttons["clear"], "true" if state == "idle" else "false")
        self.assertTrue(set(self.reviewed_actions()) <= {"start", "cancel", "clear"})
        self.state = "idle"
        self.needs_clearing = False
        run = self.run_host("panel", "print", input="quit\n")
        self.assertEqual(self.panel_buttons(run.stdout)["print"], "true")
        self.assertNotIn("clear", self.panel_buttons(run.stdout))
        self.assertIn("start", self.reviewed_actions())

    def test_absent_controls_preserves_buttons_and_enable_rules(self):
        self.controls = None
        for state in ("idle", "printing", "paused", "attention"):
            with self.subTest(state=state):
                self.state = state
                run = self.run_host("panel", "print", input='{"id":"slice"}\nquit\n')
                self.assertEqual(self.panel_buttons(run.stdout), {
                    "slice": "true", "print": "false",
                    "pause": "true" if state == "printing" else "false",
                    "resume": "true" if state == "paused" else "false",
                    "stop": "false" if state == "idle" else "true",
                    "cancel": "true" if state == "idle" else "false",
                    "clear": "true" if state == "idle" else "false"})
                self.assertNotIn("monitoring only", run.stdout)
        self.assertEqual(set(self.reviewed_actions()), {"pause", "resume", "stop", "cancel", "clear"})
        self.state = "idle"
        self.needs_clearing = False
        run = self.run_host("panel", "print", input="quit\n")
        self.assertEqual(self.panel_buttons(run.stdout)["print"], "true")
        self.assertNotIn("clear", self.panel_buttons(run.stdout))
        self.assertIn("start", self.reviewed_actions())

    def test_profile_overrides_are_encoded(self):
        self.run_host("run", "print", "slice", '{"printer":"bench","print_profile":"Fine & slow","filament_profile":"PLA+"}')
        self.assertEqual(self.requests[-1][2], "/api/v1/jobs?printer=bench&print_profile=Fine%20%26%20slow&filament_profile=PLA%2B")

    def test_panel_estimate_and_unconfirmed_button(self):
        self.needs_clearing = False
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
            while len([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"]) < 2 and time.monotonic() < started + 8:
                time.sleep(0.05)
            self.assertEqual(len([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"]), 2)
            self.assertGreaterEqual([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"][1][0] - started, 5)
            process.stdin.write("hide\n")
            process.stdin.flush()
            time.sleep(5.3)
            self.assertEqual(len([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"]), 2)
            process.stdin.write("show\n")
            process.stdin.flush()
            time.sleep(1)
            self.assertEqual(len([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"]), 2)
            time.sleep(4.3)
            self.assertEqual(len([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"]), 3)
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
            self.assertEqual(len([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"]), 1)
            while len([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"]) < 2 and time.monotonic() < started + 33:
                time.sleep(0.1)
            self.assertEqual(len([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"]), 2)
            self.assertGreaterEqual([r for r in self.requests if r[1] == "GET" and r[2] == "/api/v1/printers"][1][0] - started, 30)
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
