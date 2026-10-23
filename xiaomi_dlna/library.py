"""An explicit, persistent allowlist of local music files. No directory serving."""

import json
import os
import stat
import threading
import uuid
from pathlib import Path


MIME_TYPES = {
    "mp3": "audio/mpeg", "flac": "audio/flac", "wav": "audio/wav",
    "m4a": "audio/mp4", "aac": "audio/aac", "ogg": "audio/ogg",
    "opus": "audio/ogg", "wma": "audio/x-ms-wma",
    "aiff": "audio/aiff", "aif": "audio/aiff",
}
MAX_UPLOAD_BYTES = 2 * 1024 * 1024 * 1024
MAX_TRACKS = 20000


class Library:
    def __init__(self, data_dir):
        self.data_dir = Path(data_dir).expanduser().resolve()
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.upload_dir = self.data_dir / "uploads"
        self.upload_dir.mkdir(exist_ok=True)
        self.index_path = self.data_dir / "library.json"
        self.lock = threading.RLock()
        self.tracks = {}
        if self.index_path.exists():
            try:
                records = json.loads(self.index_path.read_text(encoding="utf-8"))
                if not isinstance(records, list):
                    raise ValueError("invalid library")
                for track in records:
                    if (isinstance(track, dict)
                            and all(isinstance(track.get(k), str) for k in
                                    ("id", "path", "title", "filename", "format"))
                            and track["format"] in MIME_TYPES
                            and isinstance(track.get("size"), int)):
                        self.tracks[track["id"]] = track
            except (OSError, ValueError):
                # Do not silently overwrite an unreadable user library.
                raise ValueError("音乐库索引无法读取，请备份后检查 %s" % self.index_path)
        self.by_path = {track["path"]: track for track in self.tracks.values()}

    def _save(self):
        temporary = self.index_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(list(self.tracks.values()),
                                        ensure_ascii=False, indent=2), encoding="utf-8")
        os.replace(str(temporary), str(self.index_path))

    @staticmethod
    def public(track):
        return {k: track[k] for k in ("id", "title", "filename", "format", "size")}

    def list(self):
        with self.lock:
            return [self.public(t) for t in self.tracks.values()]

    def get(self, track_id):
        with self.lock:
            if track_id not in self.tracks:
                raise ValueError("这首音乐不在音乐库中，请重新导入。")
            return dict(self.tracks[track_id])

    @staticmethod
    def _open_regular(path):
        # NONBLOCK prevents a substituted FIFO from hanging before fstat can
        # reject it. NOFOLLOW binds the check to the descriptor we actually use.
        flags = (os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
                 | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_BINARY", 0))
        if not hasattr(os, "O_NOFOLLOW") and Path(path).is_symlink():
            raise OSError("不允许通过符号链接读取音乐文件")
        descriptor = os.open(str(path), flags)
        try:
            details = os.fstat(descriptor)
            if not stat.S_ISREG(details.st_mode):
                raise OSError("音乐文件必须是普通文件")
            return descriptor, details
        except BaseException:
            os.close(descriptor)
            raise

    def open_file(self, track):
        """Open only the registered regular file, never a path replacement.

        Old indexes without identity fields must be explicitly reimported.
        A rescan can authorize a legitimate replacement while retaining its ID.
        """
        if not isinstance(track, dict) or not isinstance(track.get("id"), str):
            raise ValueError("这首音乐不在音乐库中，请重新导入。")
        registered = self.get(track["id"])
        expected = tuple(registered.get(key) for key in ("st_dev", "st_ino"))
        if any(isinstance(value, bool) or not isinstance(value, int) or value < 0
               for value in expected):
            raise ValueError("这首音乐的旧索引缺少文件校验信息，请重新导入文件或扫描原文件夹。")
        try:
            descriptor, details = self._open_regular(registered["path"])
        except OSError as exc:
            raise ValueError("音乐文件已移动、替换或无法安全读取，请重新导入。") from exc
        try:
            # This also detects a changed parent directory or parent symlink:
            # the opened inode must be the one the user explicitly imported.
            if (details.st_dev, details.st_ino) != expected:
                raise ValueError("音乐文件已被替换，请重新导入文件或扫描原文件夹。")
            return os.fdopen(descriptor, "rb")
        except BaseException:
            os.close(descriptor)
            raise

    def _add(self, path, filename=None):
        path = Path(path).expanduser()
        descriptor, details = self._open_regular(path)
        try:
            path = path.resolve()
        finally:
            os.close(descriptor)
        identity = {"st_dev": details.st_dev, "st_ino": details.st_ino,
                    "size": details.st_size}
        existing = self.by_path.get(str(path))
        if existing is not None:
            # Replace the record, rather than mutating a dictionary shared with
            # the transaction snapshot. Failed saves can then restore identity.
            track = dict(existing, **identity)
            self.tracks[track["id"]] = track
            self.by_path[str(path)] = track
            return track, False
        if len(self.tracks) >= MAX_TRACKS:
            raise ValueError("音乐库最多支持 20,000 首音乐。")
        name = filename or path.name
        extension = Path(name).suffix.lower().lstrip(".")
        if extension not in MIME_TYPES:
            raise ValueError("不支持此文件格式，请选择 MP3、FLAC、WAV 等音乐文件。")
        track = {"id": uuid.uuid4().hex, "path": str(path), "title": Path(name).stem,
                 "filename": name, "format": extension, **identity}
        self.tracks[track["id"]] = track
        self.by_path[str(path)] = track
        return track, True

    def add_upload(self, path, filename):
        with self.lock:
            original = dict(self.tracks)
            try:
                track, _ = self._add(path, filename)
                self._save()
            except Exception:
                self.tracks = original
                self.by_path = {track["path"]: track for track in original.values()}
                raise
            return self.public(track)

    def scan(self, directory):
        if not isinstance(directory, str) or not directory.strip():
            raise ValueError("请输入本机音乐文件夹的完整路径。")
        root = Path(directory).expanduser().resolve()
        if not root.is_dir():
            raise ValueError("文件夹不存在或无法访问，请检查路径。")
        added = skipped = 0
        # Scans and uploads share one writer; readers see a coherent library.
        with self.lock:
            original = dict(self.tracks)
            def on_error(_):
                nonlocal skipped
                skipped += 1
            try:
                for folder, dirs, names in os.walk(str(root), onerror=on_error, followlinks=False):
                    dirs[:] = sorted(d for d in dirs if not d.startswith(".")
                                     and not (Path(folder) / d).is_symlink())
                    for name in sorted(names, key=str.casefold):
                        path = Path(folder) / name
                        if path.suffix.lower().lstrip(".") not in MIME_TYPES:
                            continue
                        if path.is_symlink() or not path.is_file():
                            skipped += 1
                            continue
                        try:
                            _, fresh = self._add(path)
                            added += int(fresh)
                        except OSError:
                            skipped += 1
                self._save()
            except Exception:
                self.tracks = original
                self.by_path = {track["path"]: track for track in original.values()}
                raise
            return {"added": added, "total": len(self.tracks), "skipped": skipped}
