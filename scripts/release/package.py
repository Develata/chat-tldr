"""Package only explicit public files, and emit a SHA-256 sidecar."""

import argparse
import hashlib
from pathlib import Path
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--gui-binary", type=Path, help="Include GUI beside CLI (Windows only)")
    parser.add_argument("--version", required=True)
    parser.add_argument("--target", choices=["windows-x86_64", "linux-x86_64-musl"], required=True)
    parser.add_argument("--out", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    windows = args.target.startswith("windows")
    if args.gui_binary and not windows:
        parser.error("GUI bundles currently support windows-x86_64 only")
    product = "chat-tldr-gui" if args.gui_binary else "chat-tldr"
    name = f"{product}-{args.version}-{args.target}"
    args.out.mkdir(parents=True, exist_ok=True)
    files = [(args.binary, "chat-tldr.exe" if windows else "chat-tldr")]
    if args.gui_binary:
        files += [(args.gui_binary, "chat-tldr-gui.exe")]
        files += [(ROOT / name, name) for name in
                  ("apps/gui/README.md", "apps/gui/assets/fonts/OFL.txt", "apps/gui/assets/fonts/README.md")]
    files += [(ROOT / entry, entry) for entry in
              ("LICENSE", "README.md", "config.example.toml", "docs/DOCKER.md", "docs/RELEASING.md")]
    archive = args.out / (name + (".zip" if windows else ".tar.gz"))
    if archive.exists():
        raise FileExistsError(archive)
    if windows:
        with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as output:
            for source, destination in files:
                output.write(source, f"{name}/{destination}")
    else:
        with tarfile.open(archive, "w:gz") as output:
            for source, destination in files:
                info = output.gettarinfo(str(source), f"{name}/{destination}")
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mode = 0o755 if destination == "chat-tldr" else 0o644
                with source.open("rb") as stream:
                    output.addfile(info, stream)
    with archive.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
    print(f"Packaged {archive.name} ({archive.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
