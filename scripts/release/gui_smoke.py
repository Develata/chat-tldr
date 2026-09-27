"""Check a GUI/CLI bundle; optionally render native windows using synthetic data.

Only the Python standard library is required. On Linux use xvfb-run and Mesa.
--package-only checks extraction, versions and CLI I/O, not native rendering.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import struct
import subprocess
import tempfile
import zipfile
import zlib

ROOT = Path(__file__).resolve().parents[2]
CHAT = "qq:group:synthetic-study"


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(command, *, cwd, env, timeout=60):
    result = subprocess.run([str(value) for value in command], cwd=cwd, env=env,
                            capture_output=True, text=True, encoding="utf-8",
                            errors="replace", timeout=timeout,
                            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    check(result.returncode == 0,
          f"{command[0]} exited {result.returncode}: {result.stderr[-4000:]}")
    return result


@contextmanager
def binaries(options):
    if options.archive is None:
        check(options.gui is not None and options.cli is not None, "provide --gui and --cli")
        yield options.gui.resolve(), options.cli.resolve()
        return
    archive = options.archive.resolve()
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    sidecar = archive.with_name(archive.name + ".sha256").read_text(encoding="utf-8").strip()
    check(sidecar == f"{digest}  {archive.name}", "GUI archive checksum mismatch")
    with tempfile.TemporaryDirectory(prefix="chat-tldr-gui-package-") as temporary:
        root = Path(temporary)
        with zipfile.ZipFile(archive) as bundle:
            names = bundle.namelist()
            check(len(names) == len(set(names)), "duplicate GUI archive entry")
            prefix = f"chat-tldr-gui-{options.version}-windows-x86_64"
            expected = {f"{prefix}/{name}" for name in (
                "chat-tldr.exe", "chat-tldr-gui.exe", "LICENSE", "README.md",
                "config.example.toml", "docs/DOCKER.md", "docs/RELEASING.md", "apps/gui/README.md",
                "apps/gui/assets/fonts/OFL.txt", "apps/gui/assets/fonts/README.md")}
            check(set(names) == expected, "GUI archive must contain only the explicit public file list")
            check(all(not PurePosixPath(name).is_absolute() and ".." not in PurePosixPath(name).parts
                      and "\\" not in name for name in names), "unsafe GUI archive path")
            bundle.extractall(root)
        yield root / prefix / "chat-tldr-gui.exe", root / prefix / "chat-tldr.exe"


def png_size(path):
    data = path.read_bytes()
    check(data[:8] == b"\x89PNG\r\n\x1a\n", "capture is not a PNG")
    offset, size, pixels, complete = 8, None, bytearray(), False
    while offset + 12 <= len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        end = offset + 8 + length
        check(end + 4 <= len(data), "truncated PNG chunk")
        body = data[offset + 8:end]
        check(zlib.crc32(kind + body) == struct.unpack_from(">I", data, end)[0], "PNG checksum mismatch")
        if kind == b"IHDR":
            size = struct.unpack_from(">II", body)
        elif kind == b"IDAT":
            pixels.extend(body)
        elif kind == b"IEND":
            complete = True
            check(end + 4 == len(data), "unexpected data after PNG")
            break
        offset = end + 4
    check(complete and size is not None and min(size) >= 500, "missing or undersized capture")
    decoded = zlib.decompress(pixels)
    check(len(decoded) > size[0] * size[1] and len(set(decoded)) > 32,
          "capture lacks expected rendered content")
    return size


def exercise(options, gui, cli, data, output, env):
    check(gui.parent == cli.parent, "GUI and CLI must be installed together")
    version = run([gui, "--version"], cwd=data, env=env).stdout.strip()
    check(version == f"chat-tldr-gui {options.version}", f"unexpected GUI version: {version}")

    def command(*args):
        result = run([cli, "--data-dir", data, *args], cwd=data, env=env)
        rows = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        check(rows and rows[-1]["event"] == "done" and
              rows[-1]["payload"]["status"] == "complete" and
              rows[-1]["payload"]["exit_code"] == 0, f"incomplete CLI command {args}")
        return rows

    rows = command("version")
    check(any(row["event"] == "ack" and
              row["payload"]["detail"]["cli_version"] == options.version for row in rows),
          "GUI and CLI versions differ")
    command("config", "init")
    command("import", ROOT / "fixtures/qce/synthetic-group.json")
    check(any(row["event"] == "chat" and row["payload"]["chat_id"] == CHAT
              and row["payload"]["unreviewed_messages"] == 3 for row in command("chats")),
          "bundled CLI failed to import the synthetic chat")
    if options.package_only:
        return {"mode": "package-only", "native_rendering": False, "cases": []}

    cases = [("inbox", 1440, 900), ("evidence", 1440, 900),
             ("decisions", 1440, 1000), ("stats", 1440, 1000),
             ("overview", 1440, 1000), ("narrow-inbox", 800, 700),
             ("narrow-stats", 800, 700), ("dark-inbox", 1280, 820),
             ("cli-inbox", 1280, 820), ("missing-cli", 800, 700)]
    checked = []
    for name, width, height in cases:
        png, receipt = output / f"{name}.png", output / f"{name}.json"
        args = [gui, "--data-dir", data, "--screenshot", png, "--smoke-report", receipt,
                "--quit-after-capture", "--width", width, "--height", height]
        demo = name not in ("cli-inbox", "missing-cli")
        if demo:
            args.extend(["--demo", "--demo-view", name.removeprefix("narrow-").removeprefix("dark-")])
        if name.startswith("dark-"):
            args.append("--dark")
        if name == "missing-cli":
            args.extend(["--cli", data / "intentionally-missing-cli"])
        result = run(args, cwd=data, env=env)
        (output / f"{name}.stderr.txt").write_text(result.stderr, encoding="utf-8")
        actual_size = png_size(png)
        report = json.loads(receipt.read_text(encoding="utf-8"))
        check(report["format_version"] == 1 and report["gui_version"] == options.version,
              "unexpected capture receipt version")
        check(actual_size == (report["image_width"], report["image_height"]), "capture dimensions differ")
        check(report["demo"] == demo, "wrong application mode")
        if name == "missing-cli":
            check(not report["ready"] and report["error"] and report["cli_version"] is None,
                  "missing CLI was silently accepted")
        else:
            check(report["ready"] and report["error"] is None, f"{name} not ready: {report}")
            if demo:
                check(report["inbox"]["counts"] == {"P0": 1, "P1": 1, "P2": 1, "P3": 1},
                      "unexpected synthetic priority counts")
            else:
                check(report["cli_version"] == options.version and report["selected_chat"] == CHAT
                      and report["inbox"]["chat_id"] == CHAT and report["inbox"]["insights"] == 0,
                      "native GUI did not load the bundled CLI's synthetic inbox")
        checked.append(name)
    return {"mode": "native", "native_rendering": True, "cases": checked}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gui", type=Path)
    parser.add_argument("--cli", type=Path)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--package-only", action="store_true")
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", type=Path, required=True)
    options = parser.parse_args()
    output = options.out.resolve()
    output.mkdir(parents=True, exist_ok=False)  # Never accept stale screenshots/receipts.
    env = {key: value for key, value in os.environ.items() if key not in (
        "TYPESAFE_API_KEY", "CHAT_TLDR_LLM_API_KEY", "CHAT_TLDR_EMBED_API_KEY")}
    with binaries(options) as (gui, cli), tempfile.TemporaryDirectory(prefix="chat-tldr-gui-smoke-") as temporary:
        summary = exercise(options, gui, cli, Path(temporary), output, env)
        summary.update(version=options.version,
                       gui_sha256=hashlib.sha256(gui.read_bytes()).hexdigest(),
                       cli_sha256=hashlib.sha256(cli.read_bytes()).hexdigest())
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(f"GUI {summary['mode']} checks passed; {len(summary['cases'])} native captures; no cloud calls")


if __name__ == "__main__":
    main()
