import socket
import threading
import unittest
import xml.etree.ElementTree as ET
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch

from xiaomi_dlna.dlna import Controller, DLNAError, discover, fetch_device


AVT = "urn:schemas-upnp-org:service:AVTransport:1"
RCS = "urn:schemas-upnp-org:service:RenderingControl:1"
SOAP = "http://schemas.xmlsoap.org/soap/envelope/"


def local_name(tag):
    return tag.rsplit("}", 1)[-1]


class RendererHandler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.server.requests.append(("GET", self.path, self.headers, None))
        if self.path == "/redirect.xml":
            self.send_response(302)
            self.send_header("Location", "http://8.8.8.8/device.xml")
            self.end_headers()
            return
        if self.path == "/entity.xml":
            body = '<?xml version="1.0"?><!DOCTYPE root [<!ENTITY test "unsafe">]><root>&test;</root>'.encode("utf-16")
        elif self.path == "/huge.xml":
            body = b"x" * (1024 * 1024 + 1)
        else:
            body = self.server.description.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/xml")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        root = ET.fromstring(body)
        action = list(root.find("{%s}Body" % SOAP))[0]
        name = local_name(action.tag)
        arguments = {local_name(child.tag): child.text or "" for child in action}
        self.server.requests.append((name, self.path, self.headers, arguments))
        if name in self.server.disconnect_actions:
            self.close_connection = True
            return
        if name in self.server.fail_actions:
            status = 500
            xml = '<s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>%s</errorCode><errorDescription>Transition not available</errorDescription></UPnPError></detail></s:Fault>' % self.server.fault_code
        else:
            status = 200
            response = {
                "GetTransportInfo": "<CurrentTransportState>PLAYING</CurrentTransportState>",
                "GetPositionInfo": "<TrackDuration>01:02:03.400</TrackDuration><RelTime>00:01:23</RelTime><TrackURI>http://127.0.0.1/music.mp3</TrackURI>",
                "GetVolume": "<CurrentVolume>28</CurrentVolume>",
            }.get(name, "")
            # Prefix the output arguments too: legal XML from stricter renderers.
            response = response.replace("<", "<u:").replace("<u:/", "</u:")
            service = RCS if name in ("GetVolume", "SetVolume") else AVT
            xml = '<u:%sResponse xmlns:u="%s">%s</u:%sResponse>' % (name, service, response, name)
        data = ('<s:Envelope xmlns:s="%s"><s:Body>%s</s:Body></s:Envelope>' % (SOAP, xml)).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/xml; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class ProtocolTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), RendererHandler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = "http://127.0.0.1:%d" % cls.server.server_port

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def setUp(self):
        self.server.requests = []
        self.server.fail_actions = set()
        self.server.disconnect_actions = set()
        self.server.fault_code = "701"
        self.server.description = '''<?xml version="1.0"?>
        <root xmlns="urn:schemas-upnp-org:device-1-0">
          <URLBase>%s/base/</URLBase>
          <device><deviceType>urn:schemas-upnp-org:device:Basic:1</deviceType>
            <friendlyName>Root gateway</friendlyName><UDN>uuid:root</UDN>
            <serviceList><service><serviceType>%s</serviceType><controlURL>/wrong-volume</controlURL></service></serviceList>
            <deviceList><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
              <friendlyName>小米 &amp; 客厅</friendlyName><UDN>uuid:speaker</UDN>
              <modelName>XiaoAI</modelName><manufacturer>Xiaomi</manufacturer>
              <serviceList>
                <service><serviceType>%s</serviceType><controlURL>av/control</controlURL></service>
                <service><serviceType>%s</serviceType><controlURL>/rc/control</controlURL></service>
              </serviceList>
            </device></deviceList>
          </device>
        </root>''' % (self.base, RCS, AVT, RCS)

    def device(self):
        return fetch_device(self.base + "/description.xml")

    def test_embedded_device_keeps_its_own_services_and_resolves_relative_urls(self):
        device = self.device()
        self.assertEqual(device.id, "uuid:speaker")
        self.assertEqual(device.name, "小米 & 客厅")
        self.assertEqual(device.model, "XiaoAI")
        self.assertEqual(device.manufacturer, "Xiaomi")
        self.assertEqual(device.address, "127.0.0.1")
        self.assertEqual(device.av_transport_url, self.base + "/base/av/control")
        self.assertEqual(device.rendering_control_url, self.base + "/rc/control")
        self.assertEqual(device.to_dict()["name"], device.name)

    def test_description_location_is_base_when_urlbase_missing(self):
        self.server.description = self.server.description.replace("<URLBase>%s/base/</URLBase>" % self.base, "")
        device = fetch_device(self.base + "/nested/description.xml")
        self.assertEqual(device.av_transport_url, self.base + "/nested/av/control")

    def test_play_sends_escaped_didl_then_play(self):
        controller = Controller(self.device())
        url = self.base + "/music/a%20b.mp3?key=x&test=y"
        controller.play(url, '音乐 <现场> & "好友"', "audio/mpeg")
        requests = [entry for entry in self.server.requests if entry[0] != "GET"]
        self.assertEqual([entry[0] for entry in requests], ["Stop", "SetAVTransportURI", "Play"])
        self.assertEqual(requests[1][2]["SOAPAction"], '"%s#SetAVTransportURI"' % AVT)
        self.assertEqual(requests[1][3]["InstanceID"], "0")
        self.assertEqual(requests[1][3]["CurrentURI"], url)
        didl = ET.fromstring(requests[1][3]["CurrentURIMetaData"])
        elements = {local_name(node.tag): node for node in didl.iter()}
        self.assertEqual(elements["title"].text, '音乐 <现场> & "好友"')
        self.assertEqual(elements["class"].text, "object.item.audioItem.musicTrack")
        self.assertEqual(elements["res"].text, url)
        self.assertEqual(elements["res"].get("protocolInfo"), "http-get:*:audio/mpeg:*")
        self.assertEqual(requests[2][3]["Speed"], "1")

    def test_controls_and_namespaced_status(self):
        controller = Controller(self.device())
        controller.pause()
        controller.resume()
        controller.stop()
        controller.seek(3723)
        controller.set_volume(42)
        requests = {entry[0]: entry for entry in self.server.requests}
        self.assertEqual(requests["Seek"][3]["Target"], "01:02:03")
        self.assertEqual(requests["Seek"][3]["Unit"], "REL_TIME")
        self.assertEqual(requests["SetVolume"][1], "/rc/control")
        self.assertEqual(requests["SetVolume"][3]["DesiredVolume"], "42")
        self.assertEqual(requests["SetVolume"][3]["Channel"], "Master")
        status = controller.get_status()
        self.assertEqual(status["transport_state"], "PLAYING")
        self.assertEqual(status["position"], 83)
        self.assertEqual(status["duration"], 3723)
        self.assertEqual(status["volume"], 28)
        self.assertEqual(status["current_uri"], "http://127.0.0.1/music.mp3")

    def test_stopped_device_can_reject_stop_without_blocking_new_playback(self):
        controller = Controller(self.device())
        self.server.fail_actions.add("Stop")
        controller.play(self.base + "/music.mp3", "Music", "audio/mpeg")
        self.assertEqual(self.server.requests[-1][0], "Play")

    def test_unexpected_stop_failure_blocks_new_playback(self):
        controller = Controller(self.device())
        self.server.fail_actions.add("Stop")
        self.server.fault_code = "501"
        with self.assertRaisesRegex(DLNAError, "501"):
            controller.play(self.base + "/music.mp3", "Music", "audio/mpeg")
        self.assertEqual(self.server.requests[-1][0], "Stop")

    def test_stop_connectivity_failure_is_not_silently_ignored(self):
        controller = Controller(self.device())
        self.server.disconnect_actions.add("Stop")
        with self.assertRaisesRegex(DLNAError, "无法连接音响"):
            controller.play(self.base + "/music.mp3", "Music", "audio/mpeg")
        self.assertEqual(self.server.requests[-1][0], "Stop")

    def test_missing_transport_is_reported_and_unknown_duration_is_zero(self):
        from xiaomi_dlna.dlna import _time_seconds
        self.server.description = self.server.description.replace(AVT, "urn:schemas-upnp-org:service:ConnectionManager:1")
        with self.assertRaisesRegex(DLNAError, "AVTransport"):
            self.device()
        self.assertEqual(_time_seconds("NOT_IMPLEMENTED"), 0)
        self.assertEqual(_time_seconds("0:00:05.2/10"), 5)
        self.assertEqual(_time_seconds("27:00:03"), 97203)

    def test_ssdp_search_fallback_location_and_device_deduplication(self):
        base = self.base

        class SSDPSocket:
            def __init__(self):
                self.sent = []
                self.responses = iter([
                    (b"NOTIFY * HTTP/1.1\r\nLocation: " + base.encode() + b"/ignore.xml\r\n\r\n", ("127.0.0.1", 1900)),
                    (b"HTTP/1.1 200 OK\r\nLOCATION: " + base.encode() + b"/description.xml\r\n\r\n", ("8.8.8.8", 1900)),
                    (b"HTTP/1.1 200 OK\r\nLOCATION: " + base.encode() + b"/description.xml\r\n\r\n", ("127.0.0.1", 1900)),
                    (b"HTTP/1.1 200 OK\r\nlocation: " + base.encode() + b"/description.xml\r\n\r\n", ("127.0.0.1", 1900)),
                    (b"HTTP/1.1 200 OK\r\nLocation: " + base.encode() + b"/description.xml?second\r\n\r\n", ("127.0.0.1", 1900)),
                    (b"HTTP/1.1 200 OK\r\nLocation: http://8.8.8.8/device.xml\r\n\r\n", ("127.0.0.1", 1900)),
                ])

            def __enter__(self):
                return self

            def __exit__(self, *args):
                pass

            def setsockopt(self, *args):
                pass

            def bind(self, address):
                self.bound = address

            def settimeout(self, timeout):
                self.timeout = timeout

            def sendto(self, data, address):
                self.sent.append((data, address))

            def recvfrom(self, size):
                try:
                    return next(self.responses)
                except StopIteration:
                    raise socket.timeout()

        fake = SSDPSocket()
        original_socket = socket.socket

        def socket_factory(*args, **kwargs):
            if len(args) >= 2 and args[1] == socket.SOCK_DGRAM:
                return fake
            return original_socket(*args, **kwargs)

        with patch("socket.socket", side_effect=socket_factory):
            devices = discover(timeout=0.1, interface="127.0.0.1")
        self.assertEqual([device.id for device in devices], ["uuid:speaker"])
        self.assertEqual(fake.bound, ("127.0.0.1", 0))
        self.assertEqual(len(fake.sent), 2)
        self.assertIn(b"ST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n", fake.sent[0][0])
        self.assertIn(b"ST: ssdp:all\r\n", fake.sent[1][0])
        self.assertTrue(all(address == ("239.255.255.250", 1900) for _, address in fake.sent))
        self.assertEqual(len(self.server.requests), 2)

    def test_soap_fault_is_useful_and_play_is_not_sent_after_failed_uri(self):
        controller = Controller(self.device())
        self.server.fail_actions.add("SetAVTransportURI")
        with self.assertRaisesRegex(DLNAError, "701.*Transition not available"):
            controller.play(self.base + "/music.mp3", "Music", "audio/mpeg")
        self.assertNotIn("Play", [entry[0] for entry in self.server.requests])

    def test_optional_position_and_volume_failures_preserve_transport_status(self):
        controller = Controller(self.device())
        self.server.fail_actions.update(("GetPositionInfo", "GetVolume"))
        status = controller.get_status()
        self.assertEqual(status["transport_state"], "PLAYING")
        self.assertEqual(status["position"], 0)
        self.assertIsNone(status["volume"])
        self.assertEqual(len(status["warnings"]), 2)

    def test_optional_volume_service_is_not_borrowed_from_parent(self):
        self.server.description = self.server.description.replace('<service><serviceType>%s</serviceType><controlURL>/rc/control</controlURL></service>' % RCS, "")
        controller = Controller(self.device())
        self.assertEqual(controller.device.rendering_control_url, "")
        self.assertIsNone(controller.get_status()["volume"])
        with self.assertRaises(DLNAError):
            controller.set_volume(10)

    def test_invalid_seek_and_volume_never_reach_device(self):
        controller = Controller(self.device())
        for value in (-1, float("nan"), float("inf"), True, "12"):
            with self.subTest(seek=value), self.assertRaises(DLNAError):
                controller.seek(value)
        for value in (-1, 101, 1.5, True, "12"):
            with self.subTest(volume=value), self.assertRaises(DLNAError):
                controller.set_volume(value)
        self.assertEqual(len(self.server.requests), 1)

    def test_public_multicast_unspecified_and_authenticated_urls_are_rejected(self):
        for url in ("http://8.8.8.8/device", "http://239.255.255.250/device", "http://0.0.0.0/device", "http://[::]/device", "http://[ff02::c]/device", "http://user:secret@127.0.0.1/device", "file:///tmp/device", "http://192.0.2.1/device"):
            with self.subTest(url=url), self.assertRaises(DLNAError):
                fetch_device(url)

    def test_redirect_xml_entities_and_large_bodies_are_rejected(self):
        for path in ("/redirect.xml", "/entity.xml", "/huge.xml"):
            with self.subTest(path=path), self.assertRaises(DLNAError):
                fetch_device(self.base + path)

    def test_untrusted_control_url_is_rejected(self):
        self.server.description = self.server.description.replace("av/control", "http://8.8.8.8/control")
        with self.assertRaises(DLNAError):
            self.device()

    def test_local_dns_is_pinned_and_environment_proxy_is_ignored(self):
        original = socket.getaddrinfo
        calls = []

        def lookup(host, port, *args, **kwargs):
            if host == "speaker.invalid":
                calls.append(host)
                if len(calls) > 1:
                    return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("8.8.8.8", port))]
                return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("127.0.0.1", port))]
            return original(host, port, *args, **kwargs)

        with patch("socket.getaddrinfo", side_effect=lookup), patch.dict("os.environ", {"HTTP_PROXY": "http://127.0.0.1:1", "http_proxy": "http://127.0.0.1:1", "NO_PROXY": "", "no_proxy": ""}):
            device = fetch_device("http://speaker.invalid:%d/description.xml" % self.server.server_port)
        self.assertEqual(device.id, "uuid:speaker")
        self.assertEqual(len(calls), 1)
        self.assertEqual(self.server.requests[0][2]["Host"], "speaker.invalid:%d" % self.server.server_port)


if __name__ == "__main__":
    unittest.main()
