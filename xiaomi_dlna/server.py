"""Local-only control surface and LAN-accessible, tokenized audio streaming."""

import hmac
import ipaddress
import json
import math
import re
import secrets
import socket
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlsplit

from .dlna import Controller, DLNAError, discover, fetch_device
from .library import Library, MAX_UPLOAD_BYTES, MIME_TYPES


class App:
    def __init__(self, data_dir, advertise_host=None, interface=None):
        self.library = Library(data_dir)
        self.advertise_host = advertise_host
        self.interface = interface
        self.port = 0
        self.control_token = secrets.token_urlsafe(32)
        self.media_token = secrets.token_urlsafe(32)
        self.devices = {}
        self.lock = threading.RLock()
        self.control_lock = threading.RLock()
        self.discovery_lock = threading.Lock()
        self.closed = threading.Event()
        self.worker = None
        self.selected_device_id = None
        self.current_track_id = None
        self.transport_state = "STOPPED"
        self.position = 0
        self.duration = 0
        self.volume = None
        self.status_error = None
        self.warnings = []
        self.queue_mode = "sequential"
        self._queue_active = False
        self._observed_playing = False
        self._current_uri = None
        self._last_sample_at = None

    def start(self):
        self.worker = threading.Thread(target=self._poll, name="speaker-status", daemon=True)
        self.worker.start()

    def close(self):
        self.closed.set()
        if self.worker:
            self.worker.join(timeout=1)

    def state(self):
        tracks = self.library.list()
        with self.lock:
            return {"tracks": tracks, "devices": [d.to_dict() for d in self.devices.values()],
                    "selected_device_id": self.selected_device_id,
                    "current_track_id": self.current_track_id,
                    "transport_state": self.transport_state,
                    "position": self.position, "duration": self.duration, "volume": self.volume,
                    "status_error": self.status_error, "warnings": list(self.warnings),
                    "queue_mode": self.queue_mode}

    def device(self):
        with self.lock:
            if self.selected_device_id not in self.devices:
                raise ValueError("请先搜索并选择一台音响。")
            return self.devices[self.selected_device_id]

    def scan_devices(self):
        if not self.discovery_lock.acquire(blocking=False):
            raise ValueError("正在搜索音响，请稍候。")
        try:
            found = discover(timeout=3.0, interface=self.interface)
            with self.lock:
                # Preserve manually added and selected devices across multicast misses.
                self.devices.update((d.id, d) for d in found)
            return {"devices": [d.to_dict() for d in found]}
        finally:
            self.discovery_lock.release()

    def add_device(self, url):
        if not isinstance(url, str) or not url.strip():
            raise ValueError("请输入音响的完整设备描述地址。")
        device = fetch_device(url.strip())
        with self.lock:
            self.devices[device.id] = device
        return self.state()

    def select(self, device_id):
        with self.control_lock:
            with self.lock:
                if device_id not in self.devices:
                    raise ValueError("音响不存在，请重新搜索。")
                if device_id == self.selected_device_id:
                    return self.state()
            # Avoid leaving a second speaker playing after changing the output.
            if self._queue_active and self.selected_device_id:
                Controller(self.device()).stop()
            with self.lock:
                self.selected_device_id = device_id
                self.current_track_id = None
                self.transport_state = "STOPPED"
                self.position = self.duration = 0
                self.volume = None
                self.status_error = None
                self.warnings = []
                self._queue_active = self._observed_playing = False
                self._current_uri = None
                self._last_sample_at = None
        return self.state()

    def media_url(self, device, track):
        address = self.advertise_host
        if not address:
            # Routing toward the actual speaker chooses the correct Wi-Fi/Ethernet NIC.
            destination = urlsplit(device.av_transport_url)
            try:
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as route:
                    route.connect((destination.hostname, destination.port or 80))
                    address = route.getsockname()[0]
            except OSError as exc:
                raise ValueError("无法确定音响可访问的本机地址，请使用 --advertise-host 指定局域网 IP。") from exc
        return "http://%s:%s/media/%s/%s.%s" % (
            address, self.port, self.media_token, track["id"], track["format"])

    def play(self, track_id):
        with self.control_lock:
            track = self.library.get(track_id)
            with self.library.open_file(track):
                pass
            device = self.device()
            with self.lock:
                # A failed URI replacement must never trigger automatic queue advancement.
                self._queue_active = self._observed_playing = False
            media_uri = self.media_url(device, track)
            Controller(device).play(media_uri, track["title"], MIME_TYPES[track["format"]])
            with self.lock:
                self.current_track_id = track_id
                self.transport_state = "TRANSITIONING"
                self.position = self.duration = 0
                self.status_error = None
                self.warnings = []
                self._queue_active = True
                self._current_uri = media_uri
                self._last_sample_at = None
        return self.state()

    def _advance(self, direction=1, automatic=False):
        tracks = self.library.list()
        ids = [t["id"] for t in tracks]
        if not ids:
            raise ValueError("请先导入本地音乐。")
        current = ids.index(self.current_track_id) if self.current_track_id in ids else -1
        if automatic and self.queue_mode == "repeat-one" and current >= 0:
            target = current
        else:
            target = current + direction
            if target >= len(ids):
                if self.queue_mode == "repeat-all":
                    target = 0
                else:
                    if not automatic:
                        Controller(self.device()).stop()
                    with self.lock:
                        self._queue_active = self._observed_playing = False
                        self.transport_state = "STOPPED"
                    return self.state()
            target = max(0, target)
        return self.play(ids[target])

    @staticmethod
    def _number(value, lower, upper):
        if (isinstance(value, bool) or not isinstance(value, (int, float))
                or not math.isfinite(value) or not lower <= value <= upper):
            raise ValueError("数值必须在 %s 到 %s 之间。" % (lower, upper))
        return int(value)

    def control(self, action, value=None):
        with self.control_lock:
            if action == "mode":
                if value not in ("sequential", "repeat-one", "repeat-all"):
                    raise ValueError("未知的播放模式。")
                with self.lock:
                    self.queue_mode = value
                return self.state()
            controller = Controller(self.device())
            if action in ("next", "previous"):
                return self._advance(1 if action == "next" else -1)
            if action == "pause":
                controller.pause()
                with self.lock:
                    self.transport_state = "PAUSED_PLAYBACK"
            elif action == "resume":
                if not self.current_track_id:
                    raise ValueError("请先选择一首音乐。")
                if self.transport_state == "STOPPED":
                    return self.play(self.current_track_id)
                controller.resume()
                with self.lock:
                    self.transport_state = "TRANSITIONING"
                    self._queue_active = True
            elif action == "stop":
                with self.lock:
                    self._queue_active = self._observed_playing = False
                controller.stop()
                with self.lock:
                    self.transport_state = "STOPPED"
                    self.position = 0
            elif action == "seek":
                if not self.current_track_id or not self.duration:
                    raise ValueError("音响暂未提供歌曲时长，无法调整进度。")
                seconds = self._number(value, 0, self.duration)
                controller.seek(seconds)
                with self.lock:
                    self.position = seconds
            elif action == "volume":
                volume = self._number(value, 0, 100)
                controller.set_volume(volume)
                with self.lock:
                    self.volume = volume
            else:
                raise ValueError("未知的播放操作。")
            with self.lock:
                self.status_error = None
        return self.state()

    def _poll(self):
        while not self.closed.wait(2):
            with self.control_lock:
                if self.closed.is_set() or not self.selected_device_id:
                    continue
                try:
                    status = Controller(self.device()).get_status()
                    if self.closed.is_set():
                        return
                    sample_at = time.monotonic()
                    with self.lock:
                        old_position, old_duration = self.position, self.duration
                        state = status.get("transport_state", "STOPPED")
                        duration = status.get("duration", 0) or old_duration
                        position = status.get("position", 0)
                        # SOAP latency is part of the poll interval. A renderer may
                        # reset RelTime to zero at EOF, so use the actual elapsed
                        # interval since the last successful sample as tolerance.
                        elapsed = sample_at - self._last_sample_at if self._last_sample_at else 0
                        end_tolerance = max(3, min(20, elapsed + 1))
                        self._last_sample_at = sample_at
                        other_source = (self._current_uri and status.get("current_uri")
                                        and status["current_uri"] != self._current_uri)
                        if other_source:
                            self._queue_active = self._observed_playing = False
                            self.current_track_id = None
                            duration = status.get("duration", 0)
                        ended = (self._queue_active and self._observed_playing
                                 and state == "STOPPED" and duration > 0
                                 and max(old_position, position) >= max(0, duration - end_tolerance))
                        if state == "PLAYING":
                            self._observed_playing = True
                        self.transport_state = state
                        self.position, self.duration = position, duration
                        self.volume = status.get("volume")
                        self.warnings = list(status.get("warnings", []))
                        if other_source:
                            self.warnings.append("音响正在播放其他来源，已暂停本地队列。")
                        self.status_error = None
                        if state == "STOPPED" and not ended:
                            self._observed_playing = False
                    if ended:
                        self._advance(automatic=True)
                except (DLNAError, OSError, ValueError) as exc:
                    with self.lock:
                        self.status_error = str(exc)
                        # Do not auto-advance from stale timing after a network outage.
                        self._observed_playing = False
                        self._last_sample_at = None


class MusicServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def server_close(self):
        self.app.close()
        super().server_close()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "XiaomiDLNA/1.0"

    @property
    def app(self):
        return self.server.app

    def log_message(self, *_):
        # Media URLs carry a secret; never put them in access logs.
        pass

    def _headers(self, code, mime, length, extra=None):
        self.send_response(code)
        self.send_header("Content-Type", mime)
        self.send_header("Content-Length", str(length))
        self.send_header("Connection", "close")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        for key, value in (extra or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.close_connection = True

    def _json(self, data, code=200):
        payload = json.dumps(data, ensure_ascii=False).encode("utf-8")
        self._headers(code, "application/json; charset=utf-8", len(payload),
                      {"Cache-Control": "no-store"})
        if self.command != "HEAD":
            self.wfile.write(payload)

    def _local(self):
        if not ipaddress.ip_address(self.client_address[0]).is_loopback:
            return False
        try:
            host = urlsplit("http://" + self.headers.get("Host", ""))
            if (host.hostname not in ("localhost", "127.0.0.1", "::1")
                    or host.port != self.app.port or host.username or host.path):
                return False
            origin = self.headers.get("Origin")
            if origin:
                source = urlsplit(origin)
                if (source.scheme != "http" or source.netloc != host.netloc
                        or source.path or source.query or source.fragment):
                    return False
        except ValueError:
            return False
        return True

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        try:
            path = urlsplit(self.path).path
            if path.startswith("/media/"):
                self._media(path)
                return
            if not self._local():
                self._json({"error": "控制页面仅允许在本机通过 localhost 或 127.0.0.1 打开。"}, 403)
                return
            if path == "/api/config":
                self._json({"control_token": self.app.control_token,
                            "formats": list(MIME_TYPES), "max_upload_bytes": MAX_UPLOAD_BYTES})
            elif path == "/api/state":
                self._json(self.app.state())
            elif path in ("/", "/index.html", "/style.css", "/app.js", "/static/style.css", "/static/app.js"):
                name = "index.html" if path == "/" else path.rsplit("/", 1)[-1]
                file = Path(__file__).parent / "static" / name
                mime = {"html": "text/html", "css": "text/css", "js": "text/javascript"}[file.suffix[1:]]
                body = file.read_bytes()
                self._headers(200, mime + "; charset=utf-8", len(body), {
                    "Cache-Control": "no-cache",
                    "Content-Security-Policy": "default-src 'self'; style-src 'self'; script-src 'self'; "
                    "img-src 'self' data:; connect-src 'self'; object-src 'none'; "
                    "base-uri 'none'; frame-ancestors 'none'; form-action 'self'"})
                if self.command != "HEAD":
                    self.wfile.write(body)
            else:
                self._json({"error": "页面不存在。"}, 404)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except ValueError:
            self._json({"error": "请求地址无效。"}, 400)
        except OSError:
            self._json({"error": "文件无法读取。"}, 404)

    def _read_json(self):
        if self.headers.get_content_type() != "application/json":
            raise ValueError("请求必须使用 JSON 格式。")
        length = self._length(65536)
        data = json.loads(self.rfile.read(length).decode("utf-8"))
        if not isinstance(data, dict):
            raise ValueError("请求内容必须为 JSON 对象。")
        return data

    def _length(self, maximum):
        if self.headers.get("Transfer-Encoding"):
            raise ValueError("请使用带 Content-Length 的请求。")
        try:
            length = int(self.headers.get("Content-Length", "-1"))
        except ValueError:
            raise ValueError("请求长度无效。")
        if not 0 <= length <= maximum:
            raise ValueError("请求过大或缺少文件长度。")
        return length

    def do_POST(self):
        try:
            if not self._local() or not hmac.compare_digest(
                    self.headers.get("X-Control-Token", ""), self.app.control_token):
                self._json({"error": "请求未授权，请从本机页面刷新后重试。"}, 403)
                return
            self.connection.settimeout(30)
            path = urlsplit(self.path).path
            if path == "/api/library/upload":
                self._json(self._upload())
                return
            body = self._read_json()
            if path == "/api/discover":
                result = self.app.scan_devices()
            elif path == "/api/devices":
                result = self.app.add_device(body.get("url"))
            elif path == "/api/select":
                result = self.app.select(body.get("device_id"))
            elif path == "/api/library/scan":
                result = self.app.library.scan(body.get("path"))
            elif path == "/api/play":
                result = self.app.play(body.get("track_id"))
            elif path == "/api/control":
                result = self.app.control(body.get("action"), body.get("value"))
            else:
                self._json({"error": "接口不存在。"}, 404)
                return
            self._json(result)
        except (ValueError, TypeError, UnicodeError) as exc:
            self._json({"error": str(exc) or "请求内容无效。"}, 400)
        except DLNAError as exc:
            self._json({"error": str(exc)}, 502)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except OSError as exc:
            self._json({"error": "无法完成操作，请检查网络、文件权限和可用磁盘空间。%s" % exc}, 500)

    def _upload(self):
        length = self._length(MAX_UPLOAD_BYTES)
        if not length:
            raise ValueError("不能导入空文件。")
        filename = unquote(self.headers.get("X-Filename", ""))
        if (not filename or len(filename) > 255 or any(ord(c) < 32 for c in filename)
                or "/" in filename or "\\" in filename):
            raise ValueError("文件名无效。")
        extension = Path(filename).suffix.lower().lstrip(".")
        if extension not in MIME_TYPES:
            raise ValueError("不支持此文件格式，请选择音乐文件。")
        path = self.app.library.upload_dir / (uuid.uuid4().hex + "." + extension)
        try:
            remaining = length
            with path.open("xb") as output:
                while remaining:
                    chunk = self.rfile.read(min(128 * 1024, remaining))
                    if not chunk:
                        raise ValueError("文件上传中断，请重新选择文件。")
                    output.write(chunk)
                    remaining -= len(chunk)
            return {"track": self.app.library.add_upload(path, filename)}
        except Exception:
            path.unlink(missing_ok=True)
            raise

    def _media(self, path):
        parts = path.split("/")
        if len(parts) != 4 or not hmac.compare_digest(parts[2], self.app.media_token):
            self._json({"error": "音乐链接不存在或已过期。"}, 404)
            return
        track_id, _, extension = parts[3].partition(".")
        try:
            track = self.app.library.get(track_id)
            if track["format"] != extension:
                raise ValueError("format mismatch")
            # Open the registered path only; never concatenate URL paths to local roots.
            source = self.app.library.open_file(track)
        except (OSError, ValueError):
            self._json({"error": "音乐文件不存在。"}, 404)
            return
        with source:
            source.seek(0, 2)
            size = source.tell()
            start, end, code = 0, size - 1, 200
            extra = {"Accept-Ranges": "bytes", "Cache-Control": "private, no-store",
                     "transferMode.dlna.org": "Streaming",
                     "contentFeatures.dlna.org": "DLNA.ORG_OP=01;DLNA.ORG_CI=0;"
                     "DLNA.ORG_FLAGS=01700000000000000000000000000000"}
            byte_range = self.headers.get("Range")
            if byte_range:
                match = re.fullmatch(r"bytes=(\d*)-(\d*)", byte_range.strip())
                valid = bool(match and any(match.groups()) and size)
                if valid:
                    left, right = match.groups()
                    if left:
                        start = int(left)
                        end = min(int(right), size - 1) if right else size - 1
                    else:
                        suffix = int(right)
                        start, end = max(0, size - suffix), size - 1
                    valid = 0 <= start <= end < size
                if not valid:
                    extra["Content-Range"] = "bytes */%s" % size
                    self._headers(416, MIME_TYPES[extension], 0, extra)
                    return
                code = 206
                extra["Content-Range"] = "bytes %s-%s/%s" % (start, end, size)
            remaining = max(0, end - start + 1)
            self._headers(code, MIME_TYPES[extension], remaining, extra)
            if self.command == "HEAD":
                return
            source.seek(start)
            while remaining:
                chunk = source.read(min(128 * 1024, remaining))
                if not chunk:
                    break
                self.wfile.write(chunk)
                remaining -= len(chunk)


def create_server(host="0.0.0.0", port=18200, data_dir=None, advertise_host=None, interface=None):
    if advertise_host:
        address = ipaddress.ip_address(advertise_host)
        if (address.version != 4 or address.is_unspecified or address.is_multicast
                or not (address.is_private or address.is_link_local or address.is_loopback)):
            raise ValueError("--advertise-host 必须是本机的局域网 IPv4 地址。")
    if interface:
        address = ipaddress.ip_address(interface)
        if address.version != 4 or address.is_multicast or address.is_unspecified:
            raise ValueError("--interface 必须是本机网卡的 IPv4 地址。")
    app = App(data_dir or Path.cwd() / ".data", advertise_host, interface)
    server = MusicServer((host, port), Handler, bind_and_activate=False)
    server.app = app
    try:
        server.server_bind()
        server.server_activate()
        app.port = server.server_address[1]
        app.start()
        return server
    except Exception:
        server.server_close()
        raise
