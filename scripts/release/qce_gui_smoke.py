"""Synthetic loopback QCE/NapCat service for the actual manager/GUI/CLI pipeline."""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading

from gui_smoke import ROOT, CHAT, check, png_size, run

TOKEN = "synthetic-gui-smoke"
QR_PREFIX = "https://example.invalid/synthetic-qq-login/"


@contextmanager
def service(scenario):
    state = {"qr": 0, "exports": 0, "errors": []}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            self.respond()

        def do_POST(self):
            self.respond()

        def respond(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            content = None
            status = 200
            qr_login = scenario in ("login", "qr", "login-timeout")
            online = not qr_login or (scenario == "login" and state["qr"] >= 2)
            if self.path != "/api/auth/login":
                expected = "synthetic-credential" if self.path.startswith("/api/QQLogin/") else TOKEN
                if self.headers.get("Authorization") != f"Bearer {expected}":
                    state["errors"].append("wrong credential")
                    status = 401
            if self.path == "/api/auth/login":
                content = {"Credential": "synthetic-credential"}
            elif self.path == "/api/system/status":
                content = {"online": online}
            elif self.path == "/api/QQLogin/CheckLoginStatus":
                content = {"isLogin": online}
            elif self.path == "/api/QQLogin/GetQQLoginQrcode":
                state["qr"] += 1
                content = {"qrcode": QR_PREFIX + str(min(state["qr"], 2))}
            elif self.path.startswith("/api/recent-contacts?"):
                content = {"contacts": [
                    {"chatType": 2, "peerUid": "synthetic-study", "name": "合成学习群"},
                    {"chatType": 2, "peerUid": "synthetic-other", "name": "另一个合成群"},
                    {"chatType": 1, "peerUid": "synthetic-person", "name": "合成联系人"}]}
            elif self.path == "/api/messages/export":
                state["exports"] += 1
                request = json.loads(body)
                if request["peer"]["peerUid"] != "synthetic-study" or request["format"] != "JSON":
                    state["errors"].append("wrong export selection")
                content = {"taskId": "synthetic-task"}
            elif self.path == "/api/tasks/synthetic-task":
                content = {"status": "failed" if scenario == "export-failure" else
                           "running" if scenario == "cancel" else "completed",
                           "progress": 100, "downloadUrl": "/download/synthetic.json"}
            elif self.path == "/download/synthetic.json":
                content = (b'{"messages":[' if scenario == "malformed" else
                           b'{"messages":[]}' if scenario == "retry-import" else
                           (ROOT / "fixtures/qce/synthetic-group.json").read_bytes())
            else:
                state["errors"].append("unexpected service route")
                status, content = 404, {}
            payload = content if isinstance(content, bytes) else json.dumps({"data": content}).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            try:
                self.wfile.write(payload)
            except (BrokenPipeError, ConnectionResetError):
                pass  # Cancellation may close an in-flight request.

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", state
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=2)


def exercise(gui, cli, manager, output, env, native):
    environment = dict(env, CHAT_TLDR_QCE_TOKEN=TOKEN, CHAT_TLDR_NAPCAT_TOKEN="synthetic-napcat")
    checked = []
    scenarios = ["connected", "missing-manager", "incompatible-manager", "login", "qr",
                 "login-timeout", "export", "export-failure", "malformed", "cancel", "retry-import"]
    if not native:
        scenarios = ["export"]
    for scenario in scenarios:
        with tempfile.TemporaryDirectory(prefix="qce-gui-smoke-") as temporary, service(scenario) as (url, state):
            data = Path(temporary)
            executable = data / "missing-manager" if scenario == "missing-manager" else cli if scenario == "incompatible-manager" else manager
            (data / "gui-state.json").write_text(json.dumps({
                "cli": str(cli), "data_dir": str(data), "qce": {
                    "executable": str(executable), "base_url": url, "napcat_url": url,
                    "docker": "", "qce_config_dir": str(data), "napcat_config_dir": str(data)}
            }), encoding="utf-8")
            if not native:
                result = run([manager, "--data-dir", data, "--base-url", url, "export", "--type", "group",
                              "--peer", "synthetic-study", "--since", "2026-09-26T00:00:00Z",
                              "--until", "2026-09-28T00:00:00Z"], cwd=data, env=environment)
                rows = [json.loads(line) for line in result.stdout.splitlines()]
                check(rows[-1]["event"] == "done" and rows[-1]["payload"]["exit_code"] == 0, "manager incomplete")
                paths = [row["payload"]["detail"]["path"] for row in rows if row["event"] == "ack"]
                check(len(paths) == 1 and Path(paths[0]).is_file(), "manager did not publish export")
                run([cli, "--data-dir", data, "import", paths[0]], cwd=data, env=environment)
            else:
                mode = "connect" if scenario in ("connected", "missing-manager", "incompatible-manager") else "export" if scenario in ("export-failure", "malformed") else scenario
                png, receipt = output / f"qce-{scenario}.png", output / f"qce-{scenario}.json"
                args = [gui, "--data-dir", data, "--qce-smoke", mode, "--screenshot", png,
                        "--smoke-report", receipt, "--quit-after-capture", "--width", "1000", "--height", "1000"]
                if scenario == "qr":
                    args[args.index("--width") + 1] = "800"
                    args[args.index("--height") + 1] = "600"
                    args.append("--dark")
                result = run(args, cwd=data, env=environment)
                check(QR_PREFIX not in result.stderr and TOKEN not in result.stderr, "sensitive data in GUI diagnostics")
                size = png_size(png)
                report_text = receipt.read_text(encoding="utf-8")
                check(QR_PREFIX not in report_text and TOKEN not in report_text, "sensitive data in receipt")
                report = json.loads(report_text)
                check(size == (report["image_width"], report["image_height"]), "wrong QCE capture dimensions")
                qce = report["qce"]
                check(qce["finished"], f"{scenario}: unfinished workflow")
                if scenario in ("missing-manager", "incompatible-manager"):
                    check(not qce["compatible"] and qce["error"], "missing or wrong manager accepted")
                elif scenario in ("connected", "login"):
                    check(qce["compatible"] and qce["contacts"] == 3 and not qce["error"], "connection did not load contacts")
                    check(not qce["qr_visible"], "QR retained after login")
                elif scenario == "qr":
                    check(qce["qr_visible"] and qce["qr_fully_visible"] and qce["qr_updates"] >= 2, "refreshed QR not fully rendered")
                elif scenario == "export":
                    check(qce["imported"] and qce["import_attempts"] == 1 and report["selected_chat"] == CHAT,
                          "GUI did not import and select the exported chat")
                elif scenario == "retry-import":
                    check(qce["exported"] and not qce["imported"] and qce["import_attempts"] == 2 and qce["error"],
                          "failed import was not retried using the retained file")
                else:
                    check(not qce["imported"] and qce["import_attempts"] == 0 and qce["error"] and not qce["qr_visible"],
                          f"{scenario}: failed operation was accepted")
                check(not qce["exported"] or scenario in ("export", "retry-import"), "failed export published a path")
            # Inspect real CLI state without opening SQLite in the harness.
            rows = [json.loads(line) for line in run([cli, "--data-dir", data, "chats"], cwd=data, env=environment).stdout.splitlines()]
            chats = [row for row in rows if row["event"] == "chat"]
            check(bool(chats) == (scenario == "export"), "unexpected database import")
            if chats:
                check(chats[0]["payload"]["unreviewed_messages"] == 3, "wrong import count")
            check(state["exports"] == (1 if scenario in ("export", "export-failure", "malformed", "cancel", "retry-import") else 0), "unexpected repeated export")
            check(not state["errors"], f"fake service rejected requests: {state['errors']}")
            if scenario == "retry-import":
                check(len(list((data / "sources/qce/exports").glob("*/messages.json"))) == 1, "retained export missing or duplicated")
            checked.append(scenario)
    return {"native_rendering": native, "cases": checked}
