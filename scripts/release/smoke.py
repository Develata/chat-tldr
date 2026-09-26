"""Exercise the shipped CLI (native or Docker) with synthetic data and a local model.

Python's standard library is only a CI/developer dependency, never an image input.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


ROOT = Path(__file__).resolve().parents[2]
CHAT = "qq:group:synthetic-study"


class Models(BaseHTTPRequestHandler):
    calls = 0

    def log_message(self, *_args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path != "/chat/completions":
            self.send_error(404)
            return
        type(self).calls += 1
        source = json.loads(request["messages"][-1]["content"])
        if "questions" in source:
            answers = {}
            for key, question in source["questions"].items():
                kind = question["type"]
                if kind == "noul":
                    answer = {"type": kind, "p_yes": 0.01 if "chitchat" in key else 0.9}
                elif kind == "score":
                    count = len(question["criteria"])
                    answer = {"type": kind, "probabilities": {
                        str(i): float(i == count - 1) for i in range(count)}}
                else:
                    choices = question["criteria"]
                    selected = "new_topic" if "new_topic" in choices else next(iter(choices))
                    answer = {"type": kind, "probabilities": {
                        choice: float(choice == selected) for choice in choices}}
                answers[key] = answer
            content = {"answers": answers}
        else:
            content = {"title": "合成报告", "summary": "合成群讨论报告提交。", "relations": [], "items": [{
                "op": "new", "existing_ref": None, "kind": "todo",
                "title": "<script>提交报告</script>", "summary": "提交合成报告。",
                "assignee": "other", "deadline_raw": "周五前", "deadline_date_guess": None,
                "evidence": [{"ref": "n1", "quote": "周五前交合成报告。"}]}]}
            if source["instruction"].startswith("Group every"):
                content["refs"] = [row["ref"] for row in source["messages"]
                                   if row["ref"].startswith("n")]
                content = {"topics": [content]}
        body = json.dumps({"choices": [{"finish_reason": "stop", "message": {
            "content": json.dumps(content, ensure_ascii=False)}}],
            "usage": {"prompt_tokens": 120, "completion_tokens": 80}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def payload(events, kind):
    return next(row["payload"] for row in events if row["event"] == kind)


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def smoke(options, directory, server):
    docker = bool(options.image)
    root = "/work" if docker else str(directory)
    fixture = ROOT / "fixtures/qce/synthetic-group.json"
    before = hashlib.sha256(fixture.read_bytes()).digest()
    (directory / "input.json").write_bytes(fixture.read_bytes())
    host = "host.docker.internal" if docker else "127.0.0.1"
    (directory / "local.toml").write_text(
        f'[llm]\nbase_url="http://{host}:{server.server_port}"\n'
        'api_key_env="RELEASE_SMOKE_KEY"\ntimeout_secs=10\n', encoding="utf-8")
    env = {key: value for key, value in os.environ.items()
           if key not in ("TYPESAFE_API_KEY", "CHAT_TLDR_LLM_API_KEY", "CHAT_TLDR_EMBED_API_KEY")}
    env["RELEASE_SMOKE_KEY"] = "synthetic-release-key"
    env["NO_PROXY"] = "127.0.0.1,localhost,host.docker.internal"
    if docker:
        # The ephemeral bind directory is synthetic, isolated, and writable by UID 10001.
        directory.chmod(0o777)
        (directory / "profile").mkdir(mode=0o777)
        (directory / "profile").chmod(0o777)
        base = ["docker", "run", "--rm", "--read-only", "--cap-drop=ALL",
                "--security-opt=no-new-privileges", "--tmpfs", "/tmp:mode=1777",
                "--add-host=host.docker.internal:host-gateway", "-e", "RELEASE_SMOKE_KEY",
                "-e", "NO_PROXY", "--mount", f"type=bind,source={directory},target=/work",
                options.image]
    else:
        base = [str(Path(options.binary).resolve())]
    base += ["--data-dir", f"{root}/profile", "--config", f"{root}/local.toml"]
    checks = 0

    def cli(*args, expected=0):
        nonlocal checks
        result = subprocess.run(base + list(args), env=env, capture_output=True,
                                encoding="utf-8", timeout=120)
        check(result.returncode == expected, f"{args}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
        events = [json.loads(line) for line in result.stdout.splitlines()]
        check(bool(events), f"{args}: empty stream")
        for i, row in enumerate(events):
            check(row["schema_version"] == "1.0" and row["seq"] == i
                  and row["run_id"] == events[0]["run_id"], f"{args}: invalid JSONL envelope")
        check(events[-1]["event"] == "done" and
              events[-1]["payload"]["exit_code"] == expected, f"{args}: incomplete stream")
        check(sum(row["event"] == "done" for row in events) == 1, "duplicate done")
        checks += 1
        return events

    version = payload(cli("version"), "ack")["detail"]
    check(version["cli_version"] == options.version, "wrong packaged CLI version")
    check("overview" in version["capabilities"]["commands"], "missing core capability")
    cli("config", "init")
    original_config = (directory / "profile/config.toml").read_bytes()
    cli("config", "init", expected=4)
    check((directory / "profile/config.toml").read_bytes() == original_config, "config overwritten")
    cli("import", f"{root}/input.json")
    repeated = cli("import", f"{root}/input.json")
    check(payload(repeated, "stats")["inserted"] == 0, "import is not idempotent")
    cli("chats")
    cli("messages", "--chat", CHAT)
    cli("doctor")
    cli("analyze", "--chat", CHAT, "--decider", "llm", "--dry-run")
    check(Models.calls == 0, "dry-run contacted model")
    analyzed = cli("analyze", "--chat", CHAT, "--decider", "llm", "--html", f"{root}/analysis.html")
    check(Models.calls >= 2, "analysis did not contact local model")
    check(payload(analyzed, "stats")["messages_analyzed"] == 2, "wrong analyzed message count")
    inbox = cli("inbox", "--chat", CHAT, "--all")
    item = next(row["payload"]["insight"] for row in inbox
                if row["event"] == "insight" and row["payload"]["insight"]["kind"] == "todo")
    check(item["priority"] == "P0" and item["assignee"] == "other"
          and item["verification_status"] == "verified", "deadline/evidence policy changed")
    html = (directory / "analysis.html").read_text(encoding="utf-8")
    check("&lt;script&gt;" in html and "<script>" not in html, "unsafe HTML")
    cli("overview", "--chat", CHAT, "--since", "2026-09-25T00:00:00+08:00",
        "--until", "2026-09-28T00:00:00+08:00", "--html", f"{root}/overview.html")
    cli("feedback", item["id"], "--useful")
    cli("resolve", item["id"], "--done")
    check(not payload(cli("resolve", item["id"], "--done"), "ack")["changed"], "resolve not idempotent")
    cli("resolve", item["id"], "--reopen")
    cli("mark-read", "--chat", CHAT, "--up-to", payload(inbox, "inbox")["view_cursor"])
    for command in ("stats", "decisions", "jev-log"):
        cli(command, "--run", analyzed[0]["run_id"])
    calls = Models.calls
    cli("analyze", "--chat", CHAT, "--decider", "llm")
    check(calls == Models.calls, "completed analysis repeated model calls")
    relations = payload(cli("relations", "--chat", CHAT), "ack")["detail"]
    check(relations["uncovered_messages"] == 0 and relations["questions"] == 0,
          "relation coverage or receipt acknowledgement semantics changed")
    check(hashlib.sha256(fixture.read_bytes()).digest() == before, "fixture modified")
    print(f"Release smoke passed: {checks} CLI invocations; local model only; {'Docker' if docker else 'native'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--binary")
    mode.add_argument("--image")
    parser.add_argument("--version", required=True)
    options = parser.parse_args()
    # Docker's gateway must reach the synthetic server; no credentials or real chat data are used.
    server = ThreadingHTTPServer(("0.0.0.0" if options.image else "127.0.0.1", 0), Models)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="chat-tldr-release-") as temporary:
            smoke(options, Path(temporary), server)
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == "__main__":
    main()
