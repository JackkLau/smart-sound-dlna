import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from xiaomi_dlna.library import Library


class LibraryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.music = self.root / "music"
        self.music.mkdir()
        self.library = Library(self.root / "data")

    def tearDown(self):
        self.temp.cleanup()

    def test_recursive_scan_deduplicates_and_survives_restart(self):
        album = self.music / "专辑"
        album.mkdir()
        song = album / "夜曲.FLAC"
        song.write_bytes(b"fLaC-test-audio")
        (self.music / "notes.txt").write_text("not audio")
        first = self.library.scan(str(self.music))
        self.assertEqual(first, {"added": 1, "total": 1, "skipped": 0})
        track = self.library.list()[0]
        self.assertEqual(track["title"], "夜曲")
        self.assertEqual(track["format"], "flac")
        self.assertNotIn("path", track)
        self.assertEqual(self.library.scan(str(self.music))["added"], 0)
        restored = Library(self.root / "data")
        self.assertEqual(restored.list(), self.library.list())
        self.assertEqual(song.read_bytes(), b"fLaC-test-audio")
        self.assertEqual(list(self.library.upload_dir.iterdir()), [])

    def test_does_not_follow_symlinks_or_hidden_directories(self):
        outside = self.root / "outside.mp3"
        outside.write_bytes(b"outside")
        (self.music / "linked.mp3").symlink_to(outside)
        hidden = self.music / ".private"
        hidden.mkdir()
        (hidden / "private.wav").write_bytes(b"private")
        result = self.library.scan(str(self.music))
        self.assertEqual(result["added"], 0)
        self.assertEqual(result["skipped"], 1)

    def test_bad_directory_and_missing_track_are_actionable(self):
        with self.assertRaisesRegex(ValueError, "文件夹不存在"):
            self.library.scan(str(self.root / "missing"))
        with self.assertRaisesRegex(ValueError, "不在音乐库"):
            self.library.get("unknown")

    def test_unreadable_index_is_preserved(self):
        self.library.index_path.write_text("not valid json")
        with self.assertRaisesRegex(ValueError, "音乐库索引无法读取"):
            Library(self.root / "data")
        self.assertEqual(self.library.index_path.read_text(), "not valid json")

    def test_upload_preserves_display_name_without_exposing_path(self):
        file = self.library.upload_dir / "opaque.mp3"
        file.write_bytes(b"test audio")
        track = self.library.add_upload(file, "雨 & 晴.mp3")
        self.assertEqual(track["title"], "雨 & 晴")
        self.assertNotIn("path", track)
        records = json.loads(self.library.index_path.read_text())
        self.assertEqual(records[0]["path"], str(file))

    def registered_song(self, content=b"original audio"):
        song = self.music / "song.mp3"
        song.write_bytes(content)
        self.library.scan(str(self.music))
        return song, self.library.get(self.library.list()[0]["id"])

    def test_safe_open_returns_seekable_audio_and_persists_file_identity(self):
        song, track = self.registered_song()
        with self.library.open_file(track) as source:
            self.assertEqual(source.read(8), b"original")
            source.seek(9)
            self.assertEqual(source.read(), b"audio")
            self.assertFalse(source.writable())
        self.assertEqual((track["st_dev"], track["st_ino"]),
                         (song.stat().st_dev, song.stat().st_ino))
        restored = Library(self.root / "data")
        with restored.open_file(restored.get(track["id"])) as source:
            self.assertEqual(source.read(), b"original audio")
        self.assertNotIn("st_ino", restored.list()[0])

    def test_replacing_registered_file_with_symlink_cannot_expose_other_content(self):
        song, track = self.registered_song()
        private = self.root / "private.txt"
        private.write_bytes(b"private contents")
        song.unlink()
        song.symlink_to(private)
        with self.assertRaisesRegex(ValueError, "重新导入"):
            self.library.open_file(track)

    def test_replaced_parent_directory_cannot_substitute_an_unregistered_file(self):
        song, track = self.registered_song()
        self.music.rename(self.root / "original-music")
        self.music.mkdir()
        song.write_bytes(b"different file at the same path")
        with self.assertRaisesRegex(ValueError, "重新导入"):
            self.library.open_file(track)

    def test_parent_directory_symlink_cannot_substitute_an_unregistered_file(self):
        song, track = self.registered_song()
        other = self.root / "other"
        other.mkdir()
        (other / song.name).write_bytes(b"private contents")
        self.music.rename(self.root / "original-music")
        self.music.symlink_to(other, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "重新导入"):
            self.library.open_file(track)

    def test_rescanning_legitimate_replacement_refreshes_identity_without_duplicate(self):
        song, old_track = self.registered_song()
        replacement = self.root / "replacement.mp3"
        replacement.write_bytes(b"new music")
        replacement.replace(song)
        with self.assertRaises(ValueError):
            self.library.open_file(old_track)
        self.assertEqual(self.library.scan(str(self.music)), {"added": 0, "total": 1, "skipped": 0})
        refreshed = self.library.get(old_track["id"])
        self.assertEqual(refreshed["id"], old_track["id"])
        self.assertEqual(refreshed["size"], 9)
        self.assertNotEqual(refreshed["st_ino"], old_track["st_ino"])
        with self.library.open_file(refreshed) as source:
            self.assertEqual(source.read(), b"new music")

    def test_failed_save_rolls_back_refreshed_identity_and_existing_upload_record(self):
        song, original = self.registered_song()
        saved = self.library.index_path.read_bytes()
        replacement = self.root / "replacement.mp3"
        replacement.write_bytes(b"new music")
        replacement.replace(song)
        for operation in (lambda: self.library.scan(str(self.music)),
                          lambda: self.library.add_upload(song, "song.mp3")):
            with patch.object(self.library, "_save", side_effect=OSError("disk full")):
                with self.assertRaises(OSError):
                    operation()
            self.assertEqual(self.library.get(original["id"]), original)
            self.assertEqual(self.library.index_path.read_bytes(), saved)
            with self.assertRaises(ValueError):
                self.library.open_file(original)
        self.library.scan(str(self.music))
        with self.library.open_file(self.library.get(original["id"])) as source:
            self.assertEqual(source.read(), b"new music")

    def test_legacy_index_needs_explicit_reimport_before_file_can_be_served(self):
        _, track = self.registered_song()
        track.pop("st_dev")
        track.pop("st_ino")
        self.library.index_path.write_text(json.dumps([track]))
        legacy = Library(self.root / "data")
        self.assertEqual(len(legacy.list()), 1)
        with self.assertRaisesRegex(ValueError, "重新导入"):
            legacy.open_file(legacy.get(track["id"]))
        self.assertEqual(legacy.scan(str(self.music))["added"], 0)
        with legacy.open_file(legacy.get(track["id"])) as source:
            self.assertEqual(source.read(), b"original audio")

    @unittest.skipUnless(hasattr(os, "mkfifo"), "requires POSIX named pipes")
    def test_replaced_named_pipe_is_rejected_without_blocking(self):
        song, track = self.registered_song()
        song.unlink()
        os.mkfifo(song)
        script = '''import sys
from xiaomi_dlna.library import Library
library = Library(sys.argv[1])
try:
    library.open_file(library.get(sys.argv[2]))
except ValueError:
    sys.exit(0)
sys.exit(1)
'''
        result = subprocess.run([sys.executable, "-B", "-c", script,
                                 str(self.root / "data"), track["id"]],
                                capture_output=True, timeout=3)
        self.assertEqual(result.returncode, 0, result.stderr.decode())


if __name__ == "__main__":
    unittest.main()
