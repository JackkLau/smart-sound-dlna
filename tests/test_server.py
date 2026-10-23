"""Public HTTP integration tests; the renderer is a real local HTTP server."""

import http.client
import ipaddress
import json
import re
import socket
import tempfile
import threading
import time
import unittest
import urllib.parse
import xml.etree.ElementTree as ET
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from xiaomi_dlna.server import create_server


class RendererHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def respond(self, content):
        body = content.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/xml; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.respond('''<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <specVersion><major>1</major><minor>0</minor></specVersion>
  <device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
    <friendlyName>测试小爱音箱</friendlyName><manufacturer>Xiaomi</manufacturer>
    <modelName>Integration renderer</modelName><UDN>uuid:test-speaker</UDN>
    <serviceList>
      <service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:AVTransport</serviceId>
        <controlURL>/transport</controlURL><eventSubURL>/events</eventSubURL>
        <SCPDURL>/avt.xml</SCPDURL></service>
      <service><serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:RenderingControl</serviceId>
        <controlURL>/rendering</controlURL><eventSubURL>/events</eventSubURL>
        <SCPDURL>/rc.xml</SCPDURL></service>
      <service><serviceType>urn:schemas-upnp-org:service:ConnectionManager:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:ConnectionManager</serviceId>
        <controlURL>/connection</controlURL><eventSubURL>/events</eventSubURL>
        <SCPDURL>/cm.xml</SCPDURL></service>
    </serviceList>
  </device>
</root>''')

    def do_POST(self):
        action_header = self.headers.get("SOAPAction", "").strip('"')
        service, action = action_header.rsplit("#", 1)
        raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        envelope = ET.fromstring(raw)
        node = next(element for element in envelope.iter()
                    if element.tag.rsplit("}", 1)[-1] == action)
        arguments = {child.tag.rsplit("}", 1)[-1]: child.text or "" for child in node}
        if action == "GetTransportInfo":
            with self.server.state_lock:
                response_delay = self.server.response_delay
            if response_delay:
                time.sleep(response_delay)
        output = {}
        with self.server.state_lock:
            self.server.actions.append((action, arguments))
            if self.server.status_offline and action == "GetTransportInfo":
                # Close without a response, like a device lost during polling.
                self.close_connection = True
                return
            if action == "SetAVTransportURI":
                self.server.media_uri = arguments["CurrentURI"]
                self.server.position = "00:00:00"
                self.server.transport_state = "STOPPED"
            elif action == "Play":
                self.server.transport_state = "PLAYING"
            elif action == "Pause":
                self.server.transport_state = "PAUSED_PLAYBACK"
            elif action == "Stop":
                self.server.transport_state = "STOPPED"
            elif action == "Seek":
                self.server.position = arguments["Target"]
            elif action == "SetVolume":
                self.server.volume = int(arguments["DesiredVolume"])
            elif action == "GetVolume":
                output = {"CurrentVolume": str(self.server.volume)}
            elif action == "GetTransportInfo":
                self.server.transport_response_times.append(time.monotonic())
                output = {"CurrentTransportState": self.server.transport_state,
                          "CurrentTransportStatus": "OK", "CurrentSpeed": "1"}
            elif action == "GetPositionInfo":
                output = {"Track": "1", "TrackDuration": "00:03:00", "TrackMetaData": "",
                          "TrackURI": self.server.media_uri, "RelTime": self.server.position,
                          "AbsTime": self.server.position, "RelCount": "0", "AbsCount": "0"}
            elif action == "GetProtocolInfo":
                output = {"Source": "", "Sink": "http-get:*:audio/mpeg:*"}
        response = ET.Element("{%s}%sResponse" % (service, action))
        for key, value in output.items():
            ET.SubElement(response, key).text = value
        xml = ET.tostring(response, encoding="unicode")
        self.respond('<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">'
                     '<s:Body>' + xml + '</s:Body></s:Envelope>')


class ServerIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.data_dir = self.root / "data"
        self.renderer = None
        self.server = None
        self.start_server()

    def tearDown(self):
        self.stop_server()
        if self.renderer:
            self.renderer.shutdown()
            self.renderer.server_close()
            self.renderer_thread.join(timeout=3)
        self.temporary.cleanup()

    def start_server(self):
        self.server = create_server(host="127.0.0.1", port=0, data_dir=self.data_dir,
                                    advertise_host="127.0.0.1")
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.port = self.server.server_address[1]
        status, _, raw = self.request("GET", "/api/config")
        self.assertEqual(status, 200)
        self.config = json.loads(raw)
        self.token = self.config["control_token"]

    def stop_server(self):
        if self.server:
            self.server.shutdown()
            self.server.server_close()
            self.thread.join(timeout=3)
            self.assertFalse(self.thread.is_alive())
            self.assertFalse(self.server.app.worker.is_alive())
            self.server = None

    def request(self, method, path, body=None, headers=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=5)
        try:
            connection.request(method, path, body=body, headers=headers or {})
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def api(self, path, payload=None, expected=200):
        if payload is None:
            status, _, body = self.request("GET", path)
        else:
            status, _, body = self.request("POST", path, json.dumps(payload).encode(),
                {"Content-Type": "application/json", "X-Control-Token": self.token})
        self.assertEqual(status, expected, body.decode(errors="replace"))
        return json.loads(body)

    def upload(self, name="歌曲.mp3", data=b"ID3\x00local-audio-content"):
        status, _, body = self.request("POST", "/api/library/upload", data,
            {"Content-Type": "application/octet-stream", "X-Control-Token": self.token,
             "X-Filename": urllib.parse.quote(name)})
        self.assertEqual(status, 200, body.decode(errors="replace"))
        return json.loads(body)["track"]

    def add_renderer(self):
        self.renderer = ThreadingHTTPServer(("127.0.0.1", 0), RendererHandler)
        self.renderer.state_lock = threading.Lock()
        self.renderer.actions = []
        self.renderer.media_uri = ""
        self.renderer.transport_state = "STOPPED"
        self.renderer.position = "00:00:00"
        self.renderer.volume = 25
        self.renderer.status_offline = False
        self.renderer.response_delay = 0
        self.renderer.transport_response_times = []
        self.renderer_thread = threading.Thread(target=self.renderer.serve_forever, daemon=True)
        self.renderer_thread.start()
        url = "http://127.0.0.1:%d/description.xml" % self.renderer.server_address[1]
        state = self.api("/api/devices", {"url": url})
        self.assertEqual(state["devices"][0]["name"], "测试小爱音箱")
        self.api("/api/select", {"device_id": state["devices"][0]["id"]})

    def wait_state(self, predicate, timeout=8):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            state = self.api("/api/state")
            if predicate(state):
                return state
            time.sleep(0.05)
        self.fail("播放器状态未在限定时间内变化：%r" % state)

    def playing_uri(self, track):
        self.add_renderer()
        state = self.api("/api/play", {"track_id": track["id"]})
        self.assertEqual(state["current_track_id"], track["id"])
        with self.renderer.state_lock:
            return self.renderer.media_uri

    def test_config_and_local_control_boundary(self):
        self.assertIn("mp3", self.config["formats"])
        self.assertGreaterEqual(len(self.token), 24)
        self.assertGreater(self.config["max_upload_bytes"], 1024)
        status, _, body = self.request("GET", "/")
        self.assertEqual(status, 200)
        self.assertIn(b"<!", body)
        assets = re.findall(rb'(?:href|src)="([^"#]+\.(?:css|js))"', body)
        self.assertGreaterEqual(len(assets), 2)
        for asset in assets:
            asset_status, _, asset_body = self.request("GET", asset.decode())
            self.assertEqual(asset_status, 200, asset)
            self.assertTrue(asset_body)
        for headers in ({}, {"X-Control-Token": "wrong"},
                        {"X-Control-Token": self.token, "Origin": "https://foreign.example"}):
            status, _, _ = self.request("POST", "/api/library/scan", b"{}", headers)
            self.assertEqual(status, 403)
        for path in ("/", "/api/config", "/api/state"):
            status, _, _ = self.request("GET", path, headers={"Host": "attacker.example"})
            self.assertEqual(status, 403)
        status, _, _ = self.request("GET", "/api/config",
                                    headers={"Host": "localhost:%d" % self.port})
        self.assertEqual(status, 200)

    def test_scan_is_recursive_deduplicated_and_does_not_follow_symlinks(self):
        music = self.root / "music"
        (music / "album").mkdir(parents=True)
        (music / ".hidden").mkdir()
        (music / "one.MP3").write_bytes(b"one")
        (music / "album" / "two.flac").write_bytes(b"two")
        (music / ".hidden" / "hidden.mp3").write_bytes(b"hidden")
        (music / "notes.txt").write_text("private notes")
        external = self.root / "external.mp3"
        external.write_bytes(b"external")
        (music / "linked.mp3").symlink_to(external)
        result = self.api("/api/library/scan", {"path": str(music)})
        self.assertEqual(result["added"], 2)
        self.assertEqual(result["total"], 2)
        self.assertEqual(self.api("/api/library/scan", {"path": str(music)})["added"], 0)
        tracks = self.api("/api/state")["tracks"]
        self.assertEqual({track["filename"] for track in tracks}, {"one.MP3", "two.flac"})
        self.assertTrue(all("path" not in track for track in tracks))
        self.assertEqual((music / "one.MP3").read_bytes(), b"one")
        self.assertEqual(list((self.data_dir / "uploads").iterdir()), [])

    def test_actual_non_loopback_client_cannot_read_control_token(self):
        try:
            # UDP connect only chooses a local route; it sends no packets.
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as route:
                route.connect(("192.0.2.1", 9))
                address = route.getsockname()[0]
            if ipaddress.ip_address(address).is_loopback:
                self.skipTest("此环境没有可使用的非回环 IPv4 地址")
            connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=5,
                                                     source_address=(address, 0))
            try:
                connection.request("GET", "/api/config")
                self.assertEqual(connection.sock.getsockname()[0], address)
                response = connection.getresponse()
                status, body = response.status, response.read()
            finally:
                connection.close()
        except OSError as exc:
            self.skipTest("此环境无法从非回环地址建立本机连接：%s" % exc)
        self.assertEqual(status, 403)
        self.assertNotIn(self.token.encode(), body)

    def test_uploaded_library_survives_restart(self):
        track = self.upload()
        self.assertEqual(track["filename"], "歌曲.mp3")
        self.assertEqual(track["title"], "歌曲")
        self.assertTrue((self.data_dir / "library.json").is_file())
        self.assertEqual(len(list((self.data_dir / "uploads").iterdir())), 1)
        old_media_path = urllib.parse.urlsplit(self.playing_uri(track)).path
        old_token = self.token
        self.stop_server()
        self.start_server()
        self.assertEqual(self.api("/api/state")["tracks"], [track])
        self.assertNotEqual(self.token, old_token)
        self.assertEqual(self.request("GET", old_media_path)[0], 404)

    def test_invalid_requests_have_json_errors(self):
        for path, payload in (("/api/library/scan", {"path": str(self.root / "missing")}),
                              ("/api/play", {"track_id": "missing"}),
                              ("/api/select", {"device_id": "missing"}),
                              ("/api/control", {"action": "unsupported"})):
            status, _, body = self.request("POST", path, json.dumps(payload),
                {"Content-Type": "application/json", "X-Control-Token": self.token})
            self.assertGreaterEqual(status, 400)
            self.assertLess(status, 500)
            self.assertTrue(json.loads(body)["error"])
        status, _, body = self.request("POST", "/api/library/upload", b"not music",
            {"X-Control-Token": self.token, "X-Filename": "notes.txt"})
        self.assertEqual(status, 400)
        self.assertTrue(json.loads(body)["error"])
        self.assertEqual(self.api("/api/state")["tracks"], [])
        for filename in ("../escape.mp3", "subdir/song.mp3", "bad\\song.mp3"):
            status, _, body = self.request("POST", "/api/library/upload", b"audio",
                {"X-Control-Token": self.token, "X-Filename": urllib.parse.quote(filename)})
            self.assertEqual(status, 400)
            self.assertTrue(json.loads(body)["error"])
        status, _, body = self.request("POST", "/api/library/scan", b"{invalid",
            {"Content-Type": "application/json", "X-Control-Token": self.token})
        self.assertEqual(status, 400)
        self.assertTrue(json.loads(body)["error"])

    def test_playback_sets_a_fetchable_uri_before_play(self):
        data = b"ID3\x00" + bytes(range(256))
        track = self.upload(data=data)
        uri = self.playing_uri(track)
        parsed = urllib.parse.urlsplit(uri)
        self.assertEqual(parsed.hostname, "127.0.0.1")
        self.assertEqual(parsed.port, self.port)
        self.assertTrue(parsed.path.startswith("/media/"))
        self.assertTrue(parsed.path.endswith(".mp3"))
        with self.renderer.state_lock:
            mutations = [(action, args) for action, args in self.renderer.actions
                         if action in ("SetAVTransportURI", "Play")]
        self.assertEqual([action for action, _ in mutations[:2]], ["SetAVTransportURI", "Play"])
        metadata = mutations[0][1]["CurrentURIMetaData"]
        self.assertIn(track["title"], metadata)
        status, headers, body = self.request("GET", parsed.path,
            headers={"getcontentFeatures.dlna.org": "1", "transferMode.dlna.org": "Streaming"})
        self.assertEqual(status, 200)
        self.assertEqual(body, data)
        self.assertEqual(int(headers["Content-Length"]), len(data))
        self.assertEqual(headers["Content-Type"], "audio/mpeg")
        lowered = {key.lower(): value for key, value in headers.items()}
        self.assertIn("contentfeatures.dlna.org", lowered)
        self.assertEqual(lowered["accept-ranges"], "bytes")
        status, _, body = self.request("GET", parsed.path,
                                      headers={"Host": "192.168.1.20:%d" % self.port})
        self.assertEqual((status, body), (200, data))

    def test_media_head_ranges_and_private_file_boundary(self):
        data = b"0123456789abcdef"
        track = self.upload(data=data)
        path = urllib.parse.urlsplit(self.playing_uri(track)).path
        status, headers, body = self.request("HEAD", path)
        self.assertEqual((status, body, int(headers["Content-Length"])), (200, b"", len(data)))
        for byte_range, expected, content_range in (
                ("bytes=2-5", b"2345", "bytes 2-5/16"),
                ("bytes=12-", b"cdef", "bytes 12-15/16"),
                ("bytes=-3", b"def", "bytes 13-15/16"),
                ("bytes=14-99", b"ef", "bytes 14-15/16")):
            status, headers, body = self.request("GET", path, headers={"Range": byte_range})
            self.assertEqual((status, body), (206, expected))
            self.assertEqual(headers["Content-Range"], content_range)
            self.assertEqual(int(headers["Content-Length"]), len(expected))
        for byte_range in ("bytes=16-", "bytes=8-2", "bytes=-0", "bytes=0-1,4-5"):
            status, headers, _ = self.request("GET", path, headers={"Range": byte_range})
            self.assertEqual(status, 416)
            self.assertEqual(headers["Content-Range"], "bytes */16")
        for invalid in ("/library.json", "/uploads/", "/media/wrong/%s.mp3" % track["id"],
                        "/media/../../library.json", "/media/%2e%2e/%2e%2e/library.json"):
            status, _, body = self.request("GET", invalid)
            self.assertIn(status, (403, 404))
            self.assertNotIn(str(self.data_dir).encode(), body)

    def test_pause_resume_seek_volume_and_manual_queue_controls(self):
        first, second = self.upload("first.mp3"), self.upload("second.mp3")
        self.playing_uri(first)
        self.wait_state(lambda state: state["duration"] == 180)
        self.api("/api/control", {"action": "pause"})
        self.api("/api/control", {"action": "resume"})
        self.api("/api/control", {"action": "seek", "value": 42})
        self.api("/api/control", {"action": "volume", "value": 38})
        self.assertEqual(self.api("/api/control", {"action": "next"})["current_track_id"], second["id"])
        self.assertEqual(self.api("/api/control", {"action": "previous"})["current_track_id"], first["id"])
        self.api("/api/control", {"action": "mode", "value": "repeat-all"})
        self.api("/api/control", {"action": "next"})
        wrapped = self.api("/api/control", {"action": "next"})
        self.assertEqual(wrapped["current_track_id"], first["id"])
        self.assertEqual(wrapped["queue_mode"], "repeat-all")
        with self.renderer.state_lock:
            actions = list(self.renderer.actions)
        self.assertTrue(any(action == "Pause" for action, _ in actions))
        self.assertTrue(any(action == "Seek" and args["Target"] == "00:00:42" for action, args in actions))
        self.assertTrue(any(action == "SetVolume" and args["DesiredVolume"] == "38" for action, args in actions))

    def test_natural_completion_advances_to_next_track(self):
        first, second = self.upload("first.mp3"), self.upload("second.mp3")
        self.playing_uri(first)
        with self.renderer.state_lock:
            self.renderer.position = "00:02:59"
        self.wait_state(lambda state: state["transport_state"] == "PLAYING" and state["position"] >= 179)
        with self.renderer.state_lock:
            self.renderer.transport_state = "STOPPED"
        self.wait_state(lambda state: state["current_track_id"] == second["id"])

    def test_slow_status_poll_still_recognizes_natural_completion(self):
        first, second = self.upload("first.mp3"), self.upload("second.mp3")
        self.playing_uri(first)
        with self.renderer.state_lock:
            self.renderer.position = "00:02:56"
        self.wait_state(lambda state: state["transport_state"] == "PLAYING"
                        and state["position"] == 176 and state["duration"] == 180)
        with self.renderer.state_lock:
            self.renderer.response_delay = 2.1
            self.renderer.transport_state = "STOPPED"
            self.renderer.position = "00:00:00"
        try:
            self.wait_state(lambda state: state["current_track_id"] == second["id"]
                            or (state["transport_state"] == "STOPPED"
                                and state["position"] == 0), timeout=7)
        finally:
            with self.renderer.state_lock:
                self.renderer.response_delay = 0
        with self.renderer.state_lock:
            sample_times = list(self.renderer.transport_response_times)
        self.assertGreaterEqual(sample_times[-1] - sample_times[-2], 4)
        self.wait_state(lambda state: state["current_track_id"] == second["id"], timeout=1)

    def test_explicit_stop_does_not_advance_queue(self):
        first = self.upload("first.mp3")
        self.upload("second.mp3")
        self.playing_uri(first)
        with self.renderer.state_lock:
            self.renderer.position = "00:02:59"
        self.wait_state(lambda state: state["position"] >= 179)
        self.api("/api/control", {"action": "stop"})
        time.sleep(2.5)
        state = self.api("/api/state")
        self.assertEqual(state["current_track_id"], first["id"])
        self.assertEqual(state["transport_state"], "STOPPED")

    def test_unexpected_stop_midtrack_does_not_advance_queue(self):
        first = self.upload("first.mp3")
        self.upload("second.mp3")
        self.playing_uri(first)
        with self.renderer.state_lock:
            self.renderer.position = "00:01:00"
        self.wait_state(lambda state: state["position"] == 60)
        with self.renderer.state_lock:
            self.renderer.transport_state = "STOPPED"
        state = self.wait_state(lambda state: state["transport_state"] == "STOPPED")
        self.assertEqual(state["current_track_id"], first["id"])

    def test_repeat_one_restarts_same_track_after_natural_end(self):
        first = self.upload("first.mp3")
        self.upload("second.mp3")
        self.playing_uri(first)
        self.api("/api/control", {"action": "mode", "value": "repeat-one"})
        with self.renderer.state_lock:
            self.renderer.position = "00:02:59"
        self.wait_state(lambda state: state["position"] == 179)
        with self.renderer.state_lock:
            self.renderer.transport_state = "STOPPED"
        state = self.wait_state(lambda state: state["position"] == 0)
        self.assertEqual(state["current_track_id"], first["id"])
        with self.renderer.state_lock:
            uris = [arguments["CurrentURI"] for action, arguments in self.renderer.actions
                    if action == "SetAVTransportURI"]
        self.assertEqual(len(uris), 2)
        self.assertEqual(uris[0], uris[1])

    def test_playback_from_another_source_disarms_local_queue(self):
        first = self.upload("first.mp3")
        self.upload("second.mp3")
        self.playing_uri(first)
        with self.renderer.state_lock:
            self.renderer.position = "00:02:59"
        self.wait_state(lambda state: state["position"] == 179)
        with self.renderer.state_lock:
            self.renderer.media_uri = "http://192.168.1.50/another-player.mp3"
            self.renderer.transport_state = "STOPPED"
        state = self.wait_state(lambda state: state["current_track_id"] is None)
        self.assertTrue(state["warnings"])
        with self.renderer.state_lock:
            plays = [action for action, _ in self.renderer.actions if action == "Play"]
        self.assertEqual(plays, ["Play"])

    def test_polling_connection_failure_does_not_advance_from_stale_position(self):
        first = self.upload("first.mp3")
        self.upload("second.mp3")
        self.playing_uri(first)
        with self.renderer.state_lock:
            self.renderer.position = "00:02:59"
        self.wait_state(lambda state: state["position"] == 179)
        with self.renderer.state_lock:
            self.renderer.status_offline = True
            self.renderer.transport_state = "STOPPED"
        state = self.wait_state(lambda state: bool(state["status_error"]))
        self.assertEqual(state["current_track_id"], first["id"])
        with self.renderer.state_lock:
            self.renderer.status_offline = False
        state = self.wait_state(lambda state: not state["status_error"] and state["transport_state"] == "STOPPED")
        self.assertEqual(state["current_track_id"], first["id"])
        with self.renderer.state_lock:
            plays = [action for action, _ in self.renderer.actions if action == "Play"]
        self.assertEqual(plays, ["Play"])


if __name__ == "__main__":
    unittest.main()
