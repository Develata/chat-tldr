"""Reproduce tool correctness on author-defined synthetic gold, with local model doubles.

No real model, credentials, real chat data, or model-generated gold is used.
The two live fixture messages mean: a report deadline announcement, then a receipt.
"""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import ThreadingHTTPServer

from smoke import Models, ROOT, CHAT, check


class SyntheticJev(Models):
    def do_POST(self):
        if self.path != "/v1/systemone":
            return super().do_POST()
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        answers = {}
        for key, q in request["questions"].items():
            kind = q["type"]
            if kind == "noul":
                answers[key] = {"type": kind, "noul": 0.75 if key.startswith("n1_") else 0.25}
            elif kind == "score":
                count = len(q["criteria"])
                answers[key] = {"type": kind, "score": count - 1, "confidence": 1,
                    "legend": {str(i): label for i, label in enumerate(q["criteria"])},
                    "probabilities": {str(i): float(i == count - 1) for i in range(count)}}
            else:
                chosen = "new_topic" if "new_topic" in q["criteria"] else next(iter(q["criteria"]))
                answers[key] = {"type": kind, "choice": chosen, "confidence": 1,
                    "probabilities": {k: float(k == chosen) for k in q["criteria"]}}
        body = json.dumps({"model": request["model"], "answers": answers,
            "usage": {"input_tokens": 100, "output_tokens": 20}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--eval", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    server = ThreadingHTTPServer(("127.0.0.1", 0), SyntheticJev)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    env = {k: v for k, v in os.environ.items() if k not in
           ("TYPESAFE_API_KEY", "CHAT_TLDR_LLM_API_KEY", "CHAT_TLDR_EMBED_API_KEY")}
    env.update(SYNTHETIC_EVAL_KEY="synthetic-only", NO_PROXY="127.0.0.1,localhost")

    def execute(command, path=None):
        run = subprocess.run(list(map(str, command)), env=env, capture_output=True, timeout=120)
        if path:
            path.write_bytes(run.stdout)
        check(run.returncode == 0, f"Synthetic command failed: {run.stderr.decode('utf-8', 'replace')}")
        return [json.loads(line) for line in run.stdout.decode().splitlines()]

    try:
        with tempfile.TemporaryDirectory(prefix="chat-tldr-eval-") as tmp:
            work = Path(tmp)
            config = work / "config.toml"
            config.write_text(f'[llm]\nbase_url="http://127.0.0.1:{server.server_port}"\n'
                'model="synthetic-llm"\napi_key_env="SYNTHETIC_EVAL_KEY"\n'
                f'[jev]\nbase_url="http://127.0.0.1:{server.server_port}"\n'
                'model="synthetic-jev"\napi_key_env="SYNTHETIC_EVAL_KEY"\n', encoding="utf-8")
            fixture = ROOT / "fixtures/qce/synthetic-group.json"
            scores = {}
            for strategy in ("ours", "b0"):
                run = work / strategy
                run.mkdir()
                cli = [args.cli.resolve(), "--data-dir", run / "profile", "--config", config]
                execute(cli + ["import", fixture])
                analyze = execute(cli + ["analyze", "--chat", CHAT, "--strategy", strategy], run / "analyze.jsonl")
                messages = execute(cli + ["messages", "--chat", CHAT], run / "messages.jsonl")
                execute(cli + ["inbox", "--chat", CHAT, "--all", "--include-resolved", "--include-rejected"], run / "inbox.jsonl")
                execute(cli + ["jev-log", "--run", analyze[0]["run_id"]], run / "jev.jsonl")
                if strategy == "ours":
                    gold = work / "gold"
                    gold.mkdir()
                    rows = [r["payload"] for r in messages if r["event"] == "message" and not r["payload"]["recalled"]]
                    check(len(rows) == 2 and "周五前交合成报告" in rows[0]["display_text"] and "收到" in rows[1]["display_text"], "fixture changed; review the explicit gold")
                    # These expectations are authored from the fixed source text,
                    # not taken from either system's predictions.
                    labels = [{"message_id": r["message_id"], "thread": "report", "todo": i == 0, "announcement": i == 0} for i, r in enumerate(rows)]
                    items = [{"item_id": "report-deadline", "kind": "todo", "assignee": "all", "anchors": [rows[0]["message_id"]],
                        "deadline": {"raw": "周五前", "relation": "before", "bound_date": "2026-10-02"}, "importance": "P0"},
                        {"item_id": "report-notice", "kind": "announcement", "assignee": "all", "anchors": [rows[0]["message_id"]], "deadline": None, "importance": "P0"}]
                    for name, values in (("messages", labels), ("items", items)):
                        (gold / f"{name}.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in values), encoding="utf-8")
                    execute([args.eval.resolve(), "calibrate", "--gold", gold, "--jev", run / "jev.jsonl", "--out", args.out / "calibration"])
                    calibration = json.loads((args.out / "calibration/calibration.json").read_text())
                    check(calibration["included"] == 4, "expected two labels for two messages")
                    for group in calibration["groups"]:
                        check(abs(group["ece"] - 0.25) < 1e-6 and abs(group["binary_brier"] - 0.0625) < 1e-6, "hand-computed calibration mismatch")
                execute([args.eval.resolve(), "score", "--gold", gold, "--run", run, "--out", args.out / f"{strategy}.csv"])
                with (args.out / f"{strategy}.csv").open(encoding="utf-8", newline="") as stream:
                    scores[strategy] = {r["metric"]: r for r in csv.DictReader(stream)}
            lines = ["# 合成流程验收（非真实模型效果）", "", "仅两个未撤回消息：报告通知与确认收到。gold 在脚本中按源文本明确编写；模型为本地固定回答。", "", "| 指标 | Ours | B0 |", "|---|---:|---:|"]
            for metric in ("thread_one_to_one", "thread_exact_f1", "ari", "nmi", "todo_f1", "announcement_recall", "ndcg_at_5", "snapshot_rejected_rate", "run_calls"):
                lines.append(f'| {metric} | {scores["ours"][metric]["value"]} | {scores["b0"][metric]["value"]} |')
            lines += ["", "synthetic-jev 的 Todo/Announcement 各 2 个样本；手算 ECE=0.25，Brier=0.0625。曲线仅验证概率到标注的映射和计算。", "", f"源文件 SHA-256：`{hashlib.sha256(fixture.read_bytes()).hexdigest()}`。", "", "真实 200 条消息的质量评分与 Jev 校准曲线：**待标注**。"]
            (args.out / "README.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
            print("Synthetic Ours/B0 score and Jev calibration flow passed; no quality claim.")
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


if __name__ == "__main__":
    main()
