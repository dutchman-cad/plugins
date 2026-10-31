#!/usr/bin/env python3
"""Drive an unpacked package in a real terminal against its simulated Print companion.

Usage: package_session.py <unpacked package folder>
Covers the farm overview and the physical-action prompt answered n, then y.
Starts only the built-in simulated printer.
"""
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parent))
from print_e2e import write_mesh  # noqa: E402

TIMEOUT = 60
ESCAPES = re.compile(r"\x1b(\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[ -/]*[0-~])")


class Terminal:
    """A child process attached to a pseudo-terminal, so /dev/tty or CONIN$ really answer."""

    def __init__(self, argv, env):
        self.text = ""
        self.seen = 0
        self.lock = threading.Lock()
        if os.name == "nt":
            from winpty import PtyProcess
            self.child = PtyProcess.spawn(argv, env=env, dimensions=(50, 220))
            self.write = self.child.write
            self.read = self.child.read
        else:
            import fcntl
            import pty
            import struct
            import termios
            pid, descriptor = pty.fork()
            if pid == 0:
                os.execve(argv[0], argv, env)
            fcntl.ioctl(descriptor, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 220, 0, 0))
            self.pid = pid
            self.write = lambda text: os.write(descriptor, text.encode())
            self.read = lambda: os.read(descriptor, 65536).decode(errors="replace")
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        try:
            while True:
                chunk = self.read()
                if not chunk:
                    return
                with self.lock:
                    self.text += ESCAPES.sub("", chunk).replace("\r", "")
        except (EOFError, OSError):
            return

    def expect(self, wanted):
        deadline = time.time() + TIMEOUT
        while time.time() < deadline:
            with self.lock:
                found = self.text.find(wanted, self.seen)
                if found >= 0:
                    self.seen = found + len(wanted)
                    return
            time.sleep(0.05)
        raise AssertionError(f"Did not see {wanted!r}. Output:\n{self.text[-6000:]}")

    def close(self):
        if os.name == "nt":
            self.child.terminate(force=True)
            return
        import signal
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except OSError:
            pass

    def send(self, line):
        self.write(line + "\r")


def printer(port):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/v1/printers", timeout=10) as reply:
        return json.load(reply)[0]


def state(port):
    return printer(port)["state"]


def wait_state(port, wanted):
    deadline = time.time() + TIMEOUT
    while time.time() < deadline:
        current = state(port)
        if current == wanted:
            return
        time.sleep(0.2)
    raise AssertionError(f"Printer stayed {current}, expected {wanted}")


def main():
    package = Path(sys.argv[1]).resolve()
    suffix = ".exe" if os.name == "nt" else ""
    host = str(package / f"dutchman-plugins{suffix}")
    scratch = Path(os.environ.get("PACKAGE_SESSION_SCRATCH", package))
    with tempfile.TemporaryDirectory(prefix="session-", dir=scratch) as temporary:
        folder = Path(temporary)
        mesh = folder / "selection.3mf"
        write_mesh(mesh)
        config = folder / "config.toml"
        config.write_text('[backend]\nkind = "simulated"\n\n[[printers]]\n'
                          'id = "bench"\nname = "Workbench demo"\nmodel = "Simulated"\n')
        if os.name != "nt":
            config.chmod(0o600)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        env = {k: v for k, v in os.environ.items() if not k.startswith("DUTCHMAN_PRINT_")}
        env.pop("DUTCHMAN_PLUGINS_TEST", None)
        env["DUTCHMAN_PLUGINS_DATA"] = str(folder / "host")
        panel = None
        companion = subprocess.Popen(
            [host, "companion", "--", "--config", str(config), "serve", "--listen",
             f"127.0.0.1:{port}", "--data-dir", str(folder / "jobs")], env=env)
        try:
            deadline = time.time() + TIMEOUT
            while True:
                try:
                    state(port)
                    break
                except OSError:
                    if time.time() > deadline or companion.poll() is not None:
                        raise AssertionError("The companion did not start.")
                    time.sleep(0.2)
            panel = Terminal([host, "--yes", "--local", f"dutchman-print=http://127.0.0.1:{port}",
                              "--selection-3mf", str(mesh), "panel", "print"], env)
            panel.expect("Farm overview")
            panel.expect("Workbench demo")
            print("ok: farm overview")
            panel.send("Slice selection")
            panel.expect("Estimate:")
            panel.send("Bed is clear")
            panel.expect("Type y to confirm:")
            panel.send("n")
            panel.expect("physical action cancelled")
            if not printer(port).get("needs_clearing"):
                raise AssertionError("Answering n changed the printer.")
            print("ok: answering n sent nothing")
            panel.send("Bed is clear")
            panel.expect("Type y to confirm:")
            panel.send("y")
            panel.expect("Print (id=print) — button\n")
            if printer(port).get("needs_clearing"):
                raise AssertionError("Answering y did not clear the bed.")
            print("ok: answering y cleared the bed")
            panel.send("Print")
            panel.expect("Type y to confirm:")
            panel.send("y")
            wait_state(port, "printing")
            print("ok: answering y started the simulated printer")
            panel.send("quit")
            time.sleep(1)
        finally:
            if panel is not None:
                panel.close()
            companion.terminate()
            companion.wait(timeout=20)
            time.sleep(0.5)


if __name__ == "__main__":
    main()
