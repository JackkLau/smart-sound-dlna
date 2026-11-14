"""Stage the pinned native FFmpeg. Python is only used on the build machine."""
import hashlib
import json
import os
import shutil
from pathlib import Path
import sys
import tempfile
import urllib.request
import zipfile


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def stage(target):
    root = Path(__file__).resolve().parents[1]
    spec = json.loads((root / "scripts/subtitle-tools.json").read_text())[target]
    directory = root / "src-tauri/binaries"
    directory.mkdir(exist_ok=True)
    destination = directory / ("ffmpeg-" + target + (".exe" if "windows" in target else ""))
    if destination.is_file() and digest(destination) == spec["sha256"]:
        return
    with tempfile.TemporaryDirectory(dir=directory) as work:
        archive = Path(work) / "tool.whl"
        with urllib.request.urlopen(spec["url"], timeout=60) as src, archive.open("wb") as dst:
            shutil.copyfileobj(src, dst)
        if digest(archive) != spec["wheel_sha256"]:
            raise ValueError("FFmpeg archive checksum mismatch")
        binary = Path(work) / "ffmpeg"
        with zipfile.ZipFile(archive) as package, package.open(spec["member"]) as src, binary.open("wb") as dst:
            shutil.copyfileobj(src, dst)
        if digest(binary) != spec["sha256"]:
            raise ValueError("FFmpeg binary checksum mismatch")
        binary.chmod(0o755)
        os.replace(binary, destination)


if __name__ == "__main__":
    stage(sys.argv[1])
