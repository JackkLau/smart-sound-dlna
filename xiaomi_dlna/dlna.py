"""Small, dependency-free UPnP AV control point for LAN music renderers."""

import concurrent.futures
import http.client
import ipaddress
import math
import re
import socket
import time
import xml.etree.ElementTree as ET
from dataclasses import asdict, dataclass
from typing import Dict, List, Optional, Tuple
from urllib.parse import unquote, urljoin, urlsplit


SSDP_ADDRESS = ("239.255.255.250", 1900)
_SOAP_NS = "http://schemas.xmlsoap.org/soap/envelope/"
_XML_LIMIT = 1024 * 1024
_HTTP_TIMEOUT = 4.0
_MAX_DISCOVERY_LOCATIONS = 32
_LOCAL_NETWORKS = tuple(ipaddress.ip_network(network) for network in (
    "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "127.0.0.0/8",
    "169.254.0.0/16", "fc00::/7", "fe80::/10", "::1/128",
))


class DLNAError(Exception):
    """A discovery, connection, device, or unsupported-operation failure."""


class _SOAPFault(DLNAError):
    def __init__(self, action: str, code: str, description: str):
        self.code = code
        super().__init__("音响执行 %s 失败（%s）：%s" % (
            action, code or "UPnP", description or "设备未说明原因"))


@dataclass
class Device:
    id: str
    name: str
    model: str
    manufacturer: str
    location: str
    address: str
    av_transport_url: str
    av_transport_type: str
    rendering_control_url: str = ""
    rendering_control_type: str = ""

    def to_dict(self) -> dict:
        return asdict(self)


def _local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _children(element: ET.Element, name: str) -> List[ET.Element]:
    return [child for child in element if _local_name(child.tag) == name]


def _child_text(element: ET.Element, name: str) -> str:
    children = _children(element, name)
    return (children[0].text or "").strip() if children else ""


class _NoDoctypeTreeBuilder(ET.TreeBuilder):
    def doctype(self, name, pubid, system):
        raise DLNAError("音响返回的 XML 含有不允许的 DTD 声明")


def _xml(data: bytes) -> ET.Element:
    if len(data) > _XML_LIMIT:
        raise DLNAError("音响返回的 XML 超过 1 MB 限制")
    try:
        return ET.fromstring(data, parser=ET.XMLParser(target=_NoDoctypeTreeBuilder()))
    except (ET.ParseError, ValueError) as exc:
        raise DLNAError("音响返回了无法解析的 XML") from exc


def _is_local_address(value: str) -> bool:
    try:
        address = ipaddress.ip_address(value)
    except ValueError:
        return False
    if isinstance(address, ipaddress.IPv6Address) and address.ipv4_mapped:
        address = address.ipv4_mapped
    return not (address.is_multicast or address.is_unspecified) and any(
        address.version == network.version and address in network for network in _LOCAL_NETWORKS)


def _parse_http_url(url: str):
    if not isinstance(url, str) or any(ord(char) <= 32 or ord(char) == 127 for char in url):
        raise DLNAError("设备地址不是有效的 HTTP 或 HTTPS URL")
    try:
        parsed = urlsplit(url)
        if parsed.scheme not in ("http", "https") or not parsed.hostname:
            raise ValueError("missing HTTP host")
        if parsed.username is not None or parsed.password is not None or parsed.fragment:
            raise ValueError("credentials or fragment")
        if parsed.port is not None and not 1 <= parsed.port <= 65535:
            raise ValueError("invalid port")
    except ValueError as exc:
        raise DLNAError("设备地址必须是无账号、无片段的 HTTP 或 HTTPS URL") from exc
    return parsed


def _target(url: str) -> Tuple[object, str, int, str]:
    """Resolve once and pin the connection to a checked LAN address."""
    parsed = _parse_http_url(url)
    host = unquote(parsed.hostname)
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    try:
        ipaddress.ip_address(host)
    except ValueError:
        try:
            answers = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
        except OSError as exc:
            raise DLNAError("无法解析音响地址：%s" % host) from exc
        addresses = list(dict.fromkeys(answer[4][0] for answer in answers))
    else:
        addresses = [host]
    if not addresses or any(not _is_local_address(address) for address in addresses):
        raise DLNAError("只允许连接局域网、链路本地或本机的单播设备地址")
    return parsed, host, port, addresses[0]


class _PinnedHTTPSConnection(http.client.HTTPSConnection):
    def __init__(self, host: str, port: int, address: str, timeout: float):
        super().__init__(host, port, timeout=timeout)
        self._pinned_address = address

    def connect(self):
        self.sock = socket.create_connection((self._pinned_address, self.port), self.timeout)
        self.sock = self._context.wrap_socket(self.sock, server_hostname=self.host)


def _request(url: str, method: str = "GET", body: Optional[bytes] = None,
             headers: Optional[dict] = None, timeout: float = _HTTP_TIMEOUT) -> Tuple[int, bytes]:
    parsed, host, port, address = _target(url)
    if parsed.scheme == "https":
        connection = _PinnedHTTPSConnection(host, port, address, timeout)
    else:
        connection = http.client.HTTPConnection(address, port, timeout=timeout)
    request_headers = {"Host": parsed.netloc, "Connection": "close", "Accept": "text/xml"}
    request_headers.update(headers or {})
    path = parsed.path or "/"
    if parsed.query:
        path += "?" + parsed.query
    started = time.monotonic()
    try:
        # http.client does not consult environment proxies or follow redirects.
        connection.request(method, path, body=body, headers=request_headers)
        connected_socket = connection.sock
        response = connection.getresponse()
        if 300 <= response.status < 400:
            raise DLNAError("设备地址发生重定向；请使用音响真实的局域网描述地址")
        length = response.getheader("Content-Length")
        if length is not None:
            try:
                if int(length) > _XML_LIMIT:
                    raise DLNAError("音响返回的 XML 超过 1 MB 限制")
            except ValueError as exc:
                raise DLNAError("音响返回了无效的响应长度") from exc
        chunks = []
        total = 0
        while True:
            remaining = timeout - (time.monotonic() - started)
            if remaining <= 0:
                raise DLNAError("连接音响超时，请确认音响在线且连接同一局域网")
            if connected_socket is not None:
                connected_socket.settimeout(remaining)
            chunk = response.read1(min(65536, _XML_LIMIT + 1 - total))
            if not chunk:
                break
            chunks.append(chunk)
            total += len(chunk)
            if total > _XML_LIMIT:
                raise DLNAError("音响返回的 XML 超过 1 MB 限制")
        return response.status, b"".join(chunks)
    except (OSError, http.client.HTTPException, UnicodeError, ValueError) as exc:
        raise DLNAError("无法连接音响（%s）：%s" % (host, exc)) from exc
    finally:
        connection.close()


def _fetch_devices(location: str, timeout: float = _HTTP_TIMEOUT) -> List[Device]:
    status, body = _request(location, timeout=timeout)
    if status != 200:
        raise DLNAError("获取音响描述失败：HTTP %s" % status)
    root = _xml(body)
    base = _child_text(root, "URLBase") or location
    base = urljoin(location, base)
    devices = []
    for index, element in enumerate(root.iter()):
        if _local_name(element.tag) != "device":
            continue
        services = {}
        # Only direct services belong to this device. Embedded devices have
        # their own identity, transport and rendering-control service pairing.
        for service_list in _children(element, "serviceList"):
            for service in _children(service_list, "service"):
                service_type = _child_text(service, "serviceType")
                match = re.fullmatch(r"urn:schemas-upnp-org:service:(AVTransport|RenderingControl):([1-9]\d*)", service_type)
                control = _child_text(service, "controlURL")
                if match and control:
                    kind, version = match.group(1), int(match.group(2))
                    if kind not in services or version > services[kind][0]:
                        services[kind] = (version, urljoin(base, control), service_type)
        if "AVTransport" not in services:
            continue
        _, transport_url, transport_type = services["AVTransport"]
        _, rendering_url, rendering_type = services.get("RenderingControl", (0, "", ""))
        _, transport_host, _, _ = _target(transport_url)
        if rendering_url:
            _target(rendering_url)
        devices.append(Device(
            id=_child_text(element, "UDN") or "%s#device-%s" % (location, index),
            name=_child_text(element, "friendlyName") or _child_text(element, "modelName") or "DLNA 音响",
            model=_child_text(element, "modelName"),
            manufacturer=_child_text(element, "manufacturer"),
            location=location,
            address=transport_host,
            av_transport_url=transport_url,
            av_transport_type=transport_type,
            rendering_control_url=rendering_url,
            rendering_control_type=rendering_type,
        ))
    return devices


def fetch_device(location: str) -> Device:
    """Load a root description and return its first controllable renderer."""
    devices = _fetch_devices(location)
    if not devices:
        raise DLNAError("此设备没有 DLNA AVTransport 服务；请确认该型号和固件支持 DLNA 播放")
    return devices[0]


def discover(timeout: float = 3.0, interface: Optional[str] = None) -> List[Device]:
    """Search IPv4 SSDP, then spend at most ~3 seconds resolving descriptions.

    ``interface`` is the local IPv4 address to use on a multi-adapter machine.
    Unreachable or non-renderer advertisements are ignored. Permission and
    socket failures remain actionable errors instead of an empty device list.
    """
    if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or not 0 < timeout <= 30:
        raise DLNAError("发现设备的等待时间必须在 0 到 30 秒之间")
    if interface is not None:
        try:
            interface_ip = ipaddress.IPv4Address(interface)
        except (ValueError, TypeError) as exc:
            raise DLNAError("发现网卡必须填写本机的局域网 IPv4 地址") from exc
        if not _is_local_address(str(interface_ip)):
            raise DLNAError("发现网卡必须填写本机的局域网 IPv4 地址")
    locations = []
    seen_locations = set()
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP) as sock:
            sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 2)
            if interface:
                sock.bind((interface, 0))
                sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton(interface))
            else:
                sock.bind(("", 0))
            mx = max(1, min(3, int(timeout)))
            for target in ("urn:schemas-upnp-org:device:MediaRenderer:1", "ssdp:all"):
                message = ("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\n"
                           'MAN: "ssdp:discover"\r\nMX: %s\r\nST: %s\r\n\r\n' % (mx, target))
                sock.sendto(message.encode("ascii"), SSDP_ADDRESS)
            deadline = time.monotonic() + timeout
            while len(locations) < _MAX_DISCOVERY_LOCATIONS:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                sock.settimeout(remaining)
                try:
                    packet, sender = sock.recvfrom(65535)
                except socket.timeout:
                    break
                if not _is_local_address(sender[0]):
                    continue
                lines = packet.decode("iso-8859-1").splitlines()
                if not lines or not re.match(r"^HTTP/1\.[01]\s+200(?:\s|$)", lines[0], re.IGNORECASE):
                    continue
                headers = {}
                for line in lines[1:]:
                    if ":" in line:
                        key, value = line.split(":", 1)
                        headers[key.strip().lower()] = value.strip()
                location = headers.get("location", "")
                if location and location not in seen_locations:
                    seen_locations.add(location)
                    locations.append(location)
    except OSError as exc:
        raise DLNAError("搜索音响失败：%s。请检查局域网权限、网卡和防火墙设置" % exc) from exc
    if not locations:
        return []
    description_deadline = time.monotonic() + 3.0

    def read_description(location):
        remaining = description_deadline - time.monotonic()
        if remaining <= 0:
            return []
        try:
            return _fetch_devices(location, timeout=min(2.0, remaining))
        except DLNAError:
            return []

    found = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=min(8, len(locations))) as pool:
        for devices in pool.map(read_description, locations):
            for device in devices:
                found.setdefault(device.id, device)
    return sorted(found.values(), key=lambda device: (device.name.casefold(), device.id))


def _time_seconds(value: str) -> int:
    match = re.fullmatch(r"(\d+):([0-5]\d):([0-5]\d)(?:\.\d+(?:/\d+)?)?", value.strip())
    return int(match.group(1)) * 3600 + int(match.group(2)) * 60 + int(match.group(3)) if match else 0


def _didl(url: str, title: str, mime: str) -> str:
    root = ET.Element("DIDL-Lite", {
        "xmlns": "urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/",
        "xmlns:dc": "http://purl.org/dc/elements/1.1/",
        "xmlns:upnp": "urn:schemas-upnp-org:metadata-1-0/upnp/",
    })
    item = ET.SubElement(root, "item", {"id": "0", "parentID": "-1", "restricted": "1"})
    ET.SubElement(item, "dc:title").text = title
    ET.SubElement(item, "upnp:class").text = "object.item.audioItem.musicTrack"
    ET.SubElement(item, "res", {"protocolInfo": "http-get:*:%s:*" % mime}).text = url
    return ET.tostring(root, encoding="unicode")


class Controller:
    def __init__(self, device: Device):
        self.device = device

    def _soap(self, action: str, arguments: Optional[dict] = None, rendering: bool = False) -> Dict[str, str]:
        url = self.device.rendering_control_url if rendering else self.device.av_transport_url
        service = self.device.rendering_control_type if rendering else self.device.av_transport_type
        if not url or not service:
            raise DLNAError("此音响没有提供音量控制服务" if rendering else "此音响没有提供播放控制服务")
        envelope = ET.Element("s:Envelope", {"xmlns:s": _SOAP_NS, "s:encodingStyle": "http://schemas.xmlsoap.org/soap/encoding/"})
        body = ET.SubElement(envelope, "s:Body")
        operation = ET.SubElement(body, "u:" + action, {"xmlns:u": service})
        for key, value in dict({"InstanceID": "0"}, **(arguments or {})).items():
            ET.SubElement(operation, key).text = str(value)
        payload = ET.tostring(envelope, encoding="utf-8", xml_declaration=True)
        status, data = _request(url, "POST", payload, {
            "Content-Type": 'text/xml; charset="utf-8"',
            "SOAPAction": '"%s#%s"' % (service, action),
        })
        if not data.strip():
            if 200 <= status < 300:
                return {}
            raise DLNAError("音响执行 %s 失败：HTTP %s" % (action, status))
        try:
            root = _xml(data)
        except DLNAError:
            if not 200 <= status < 300:
                raise DLNAError("音响执行 %s 失败：HTTP %s" % (action, status))
            raise
        values = {_local_name(element.tag): (element.text or "").strip() for element in root.iter()}
        if "Fault" in values:
            raise _SOAPFault(action, values.get("errorCode", ""), values.get("errorDescription") or values.get("faultstring", ""))
        if not 200 <= status < 300:
            raise DLNAError("音响执行 %s 失败：HTTP %s" % (action, status))
        return values

    def play(self, url: str, title: str, mime: str):
        _parse_http_url(url)
        if not isinstance(title, str) or not isinstance(mime, str) or not re.fullmatch(r"audio/[A-Za-z0-9.+_-]+|application/ogg", mime):
            raise DLNAError("音乐标题或音频类型无效")
        # Some renderers cannot change the URI while playing. An idle device
        # can legitimately reject Stop with 701; older firmware may use 401.
        try:
            self.stop()
        except _SOAPFault as exc:
            if exc.code not in ("401", "701"):
                raise
        self._soap("SetAVTransportURI", {"CurrentURI": url, "CurrentURIMetaData": _didl(url, title, mime)})
        self.resume()

    def pause(self):
        self._soap("Pause")

    def resume(self):
        self._soap("Play", {"Speed": "1"})

    def stop(self):
        self._soap("Stop")

    def seek(self, seconds):
        if isinstance(seconds, bool) or not isinstance(seconds, (int, float)) or not math.isfinite(seconds) or seconds < 0:
            raise DLNAError("播放位置必须是大于或等于 0 的秒数")
        seconds = int(seconds)
        self._soap("Seek", {"Unit": "REL_TIME", "Target": "%02d:%02d:%02d" % (seconds // 3600, seconds // 60 % 60, seconds % 60)})

    def set_volume(self, volume: int):
        if isinstance(volume, bool) or not isinstance(volume, int) or not 0 <= volume <= 100:
            raise DLNAError("音量必须是 0 到 100 之间的整数")
        self._soap("SetVolume", {"Channel": "Master", "DesiredVolume": volume}, rendering=True)

    def get_status(self) -> dict:
        transport = self._soap("GetTransportInfo")
        result = {"transport_state": transport.get("CurrentTransportState", "UNKNOWN").upper(), "position": 0, "duration": 0, "volume": None}
        warnings = []
        try:
            position = self._soap("GetPositionInfo")
            result["position"] = _time_seconds(position.get("RelTime", ""))
            result["duration"] = _time_seconds(position.get("TrackDuration", ""))
            if position.get("TrackURI"):
                result["current_uri"] = position["TrackURI"]
        except DLNAError as exc:
            warnings.append(str(exc))
        if self.device.rendering_control_url:
            try:
                volume = self._soap("GetVolume", {"Channel": "Master"}, rendering=True)
                result["volume"] = max(0, min(100, int(volume["CurrentVolume"])))
            except (DLNAError, ValueError, KeyError) as exc:
                warnings.append("读取音量失败：%s" % exc)
        if warnings:
            result["warnings"] = warnings
        return result
