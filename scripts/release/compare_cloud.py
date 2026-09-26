"""Manual, explicitly authorized cloud acceptance; never invoked by CI.

Raw chats, model outputs and unlabelled sheets stay in --out (use private/).
Only aggregate.json is suitable for review before publication. No gold is created.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.request import Request, urlopen
from urllib.error import HTTPError
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[2]

class TraceProxy(BaseHTTPRequestHandler):
    """Loopback-only manual diagnostic. Save bodies, never authorization headers."""
    directory = None
    counter = 0
    lock = threading.Lock()
    def log_message(self, *_args):
        pass
    def do_POST(self):
        upstream = {"/jev/v1/systemone": "https://api.typesafe.ai/v1/systemone",
                    "/llm/chat/completions": "https://api.deepseek.com/chat/completions"}.get(self.path)
        if not upstream:
            self.send_error(404)
            return
        body = self.rfile.read(int(self.headers["Content-Length"]))
        request = Request(upstream, data=body, headers={"Content-Type": "application/json", "Authorization": self.headers["Authorization"]})
        try:
            response = urlopen(request, timeout=180)
        except HTTPError as error:
            response = error
        with response:
            output = response.read()
            status = response.status
        with self.lock:
            type(self).counter += 1
            index = type(self).counter
        (self.directory / f"{index:04d}-request.json").write_bytes(body)
        (self.directory / f"{index:04d}-response.json").write_bytes(output)
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(output)))
        self.end_headers()
        self.wfile.write(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--eval", type=Path, required=True)
    parser.add_argument("--trace-proxy", action="store_true", help="Keep private provider request/response bodies for manual diagnosis")
    args = parser.parse_args()
    out = args.out.resolve()
    # This workflow contains unredacted data: keep it under the ignored private root.
    out.relative_to((ROOT / "private").resolve())
    out.mkdir(parents=True, exist_ok=False)
    config = out / "config.toml"
    shutil.copyfile(ROOT / "config.example.toml", config)
    server = None
    if args.trace_proxy:
        TraceProxy.directory = out / "provider-traces"
        TraceProxy.directory.mkdir()
        server = ThreadingHTTPServer(("127.0.0.1", 0), TraceProxy)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        config.write_text(config.read_text(encoding="utf-8").replace('"https://api.typesafe.ai"', f'"http://127.0.0.1:{server.server_port}/jev"').replace('"https://api.deepseek.com"', f'"http://127.0.0.1:{server.server_port}/llm"'), encoding="utf-8")
    original_hash = hashlib.sha256(args.source.read_bytes()).hexdigest()
    aggregate = {"executed_at": datetime.now(timezone.utc).isoformat(),
        "cli_sha256": hashlib.sha256(args.cli.read_bytes()).hexdigest(),
        "source_sha256": original_hash, "quality": "待标注", "jev_calibration": "待标注",
        "calls_unit": "logical provider calls; transport retries are not individually counted",
        "cost_unit": "USD estimated from configured rates, not provider invoice", "systems": []}
    for strategy in ("ours", "b0"):
        run = out / strategy
        run.mkdir()
        command = [str(args.cli.resolve()), "--data-dir", str(run / "profile"), "--config", str(config)]
        exits = {}

        def cli(name, extra):
            result = subprocess.run(command + extra, capture_output=True, timeout=900)
            (run / f"{name}.jsonl").write_bytes(result.stdout)
            (run / f"{name}.stderr.log").write_bytes(result.stderr)
            exits[name] = result.returncode
            return [json.loads(line) for line in result.stdout.decode("utf-8").splitlines()]

        imported = cli("import", ["import", str(args.source.resolve())])
        if exits["import"] != 0:
            raise RuntimeError("Cloud sample import failed; see private artifacts")
        chat = next(e["payload"]["detail"]["chat_ids"][0] for e in imported if e["event"] == "ack")
        events = cli("analyze", ["analyze", "--chat", chat, "--strategy", strategy,
            "--decider", "jev", "--max-steps", "256", "--budget-usd", "0.50"])
        run_id = events[0]["run_id"]
        cli("messages", ["messages", "--chat", chat])
        inbox = cli("inbox", ["inbox", "--chat", chat, "--all", "--include-resolved", "--include-rejected"])
        decisions = cli("decisions", ["decisions", "--run", run_id])
        cli("jev-log", ["jev-log", "--run", run_id])
        relations = cli("relations", ["relations", "--chat", chat])
        history = cli("stats", ["stats", "--run", run_id])
        stats = next((e["payload"] for e in events if e["event"] == "stats"), None)
        if stats is None:
            stats = next((e["payload"] for e in history if e["event"] == "stats"), {})
        usage = stats.get("usage", [])
        counts = {}
        for e in inbox:
            if e["event"] == "insight":
                status = e["payload"]["insight"]["verification_status"]
                counts[status] = counts.get(status, 0) + 1
        controller = {}
        for e in decisions:
            if e["event"] == "decision":
                method = e["payload"]["method"]
                controller[method] = controller.get(method, 0) + 1
        row = {"strategy": strategy, "exits": exits, "status": events[-1]["payload"].get("status"),
            "messages_analyzed": stats.get("messages_analyzed"), "elapsed_ms": stats.get("elapsed_ms"),
            "input_tokens": sum(u["input_tokens"] for u in usage), "output_tokens": sum(u["output_tokens"] for u in usage),
            "logical_calls": sum(u["calls"] for u in usage), "cache_hits": sum(u["cache_hits"] for u in usage),
            "estimated_cost_usd": stats.get("cost_usd"), "usage": usage, "saved_insights": counts,
            "controller_methods": controller, "warnings": [e["payload"]["code"] for e in events if e["event"] == "warning"],
            "relation_coverage": relations[0]["payload"]["detail"]}
        aggregate["systems"].append(row)
        (out / "aggregate.json").write_text(json.dumps(aggregate, indent=2, ensure_ascii=False), encoding="utf-8")
        if strategy == "ours":
            sheet = subprocess.run([str(args.eval.resolve()), "export-sheet", "--messages", str(run / "messages.jsonl"), "--out", str(out / "annotation.csv"), "--exit-code", str(exits["messages"])], capture_output=True)
            (out / "export-sheet.json").write_bytes(sheet.stdout)
            if sheet.returncode != 0:
                raise RuntimeError("Annotation export failed; see private artifacts")
        print(json.dumps({k: row[k] for k in ("strategy", "status", "messages_analyzed", "logical_calls", "input_tokens", "output_tokens", "estimated_cost_usd", "controller_methods")}), flush=True)
    if hashlib.sha256(args.source.read_bytes()).hexdigest() != original_hash:
        raise RuntimeError("Input source changed during acceptance")
    if server:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
