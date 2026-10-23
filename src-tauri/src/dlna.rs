//! Native, bounded UPnP AV discovery and control. No proxy or Python process.

use futures_util::{stream, StreamExt};
use reqwest::{redirect::Policy, Client, Method};
use roxmltree::{Document, Node, ParsingOptions};
use serde::Serialize;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::{lookup_host, UdpSocket};
use tokio::time::{self, Instant};
use url::{Host, Url};

const XML_LIMIT: usize = 1024 * 1024;
const XML_NODES_LIMIT: u32 = 20_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
const SEARCH_WINDOW: Duration = Duration::from_secs(4);
const SEARCH_RETRY: Duration = Duration::from_secs(2);
const DESCRIPTION_WINDOW: Duration = Duration::from_secs(6);
const MAX_LOCATIONS: usize = 128;
const MAX_LOCATIONS_PER_INTERFACE: usize = 64;
const SSDP_TARGETS: [&str; 3] = [
    "urn:schemas-upnp-org:device:MediaRenderer:1",
    "urn:schemas-upnp-org:service:AVTransport:1",
    "ssdp:all",
];

#[derive(Serialize, Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub manufacturer: String,
    pub location: String,
    pub address: String,
    pub av_transport_url: String,
    pub av_transport_type: String,
    pub rendering_control_url: String,
    pub rendering_control_type: String,
    pub is_xiaomi: bool,
    pub is_xiaomi_pro: bool,
    pub interface_ip: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct DiscoveryReport {
    pub devices: Vec<Device>,
    pub warnings: Vec<String>,
    pub interfaces: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct TransportStatus {
    pub transport_state: String,
    pub position: u64,
    pub duration: u64,
    /// Both time values came from a valid GetPositionInfo response. Consumers
    /// must clear completion tracking on false, even if transport is STOPPED.
    pub position_valid: bool,
    pub volume: Option<u8>,
    pub current_uri: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct NetworkInterface {
    name: String,
    ip: Ipv4Addr,
}

#[derive(Clone, Debug)]
struct Candidate {
    location: String,
    interface_ip: Ipv4Addr,
}

#[derive(Default)]
struct ScanResult {
    candidates: Vec<Candidate>,
    warnings: Vec<String>,
}

struct Target {
    url: Url,
    host: String,
    addresses: Vec<SocketAddr>,
}

/// Includes RFC1918, loopback, link-local and IPv6 unique-local unicast only.
fn is_lan_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && (ip.is_private() || ip.is_loopback() || ip.is_link_local())
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_lan_address(IpAddr::V4(mapped));
            }
            let first = ip.segments()[0];
            !ip.is_unspecified()
                && !ip.is_multicast()
                && (ip.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80)
        }
    }
}

fn parse_http_url(value: &str) -> Result<Url, String> {
    if value.chars().any(|c| c <= '\u{20}' || c == '\u{7f}') {
        return Err("设备地址包含空格或控制字符，请使用完整的 HTTP 地址。".into());
    }
    let url =
        Url::parse(value).map_err(|_| "设备地址不是有效的 HTTP 或 HTTPS URL。".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() == Some(0)
    {
        return Err("设备地址必须是无账号、无片段的 HTTP 或 HTTPS URL。".into());
    }
    // Reject literals before DNS work. Domains are checked after resolution.
    match url.host() {
        Some(Host::Ipv4(ip)) if !is_lan_address(IpAddr::V4(ip)) => Err(lan_only_error()),
        Some(Host::Ipv6(ip)) if !is_lan_address(IpAddr::V6(ip)) => Err(lan_only_error()),
        _ => Ok(url),
    }
}

fn lan_only_error() -> String {
    "只允许连接局域网、链路本地或本机的单播设备地址。".into()
}

fn host_string(url: &Url) -> String {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.to_string(),
        Some(Host::Ipv6(ip)) => ip.to_string(),
        Some(Host::Domain(host)) => host.to_string(),
        None => String::new(),
    }
}

fn validate_addresses(addresses: &[SocketAddr]) -> Result<(), String> {
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !is_lan_address(address.ip()))
    {
        return Err(lan_only_error());
    }
    Ok(())
}

async fn resolve_target(value: &str) -> Result<Target, String> {
    let url = parse_http_url(value)?;
    let host = host_string(&url);
    let port = url.port_or_known_default().ok_or_else(lan_only_error)?;
    let mut addresses = match url.host() {
        Some(Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        Some(Host::Domain(_)) => {
            time::timeout(Duration::from_secs(2), lookup_host((host.as_str(), port)))
                .await
                .map_err(|_| format!("解析音响地址 {host} 超时。"))?
                .map_err(|_| format!("无法解析音响地址 {host}。"))?
                .collect::<Vec<_>>()
        }
        None => return Err(lan_only_error()),
    };
    addresses.sort_unstable();
    addresses.dedup();
    validate_addresses(&addresses)?;
    Ok(Target {
        url,
        host,
        addresses,
    })
}

fn pinned_client(target: &Target, interface_ip: Option<Ipv4Addr>) -> Result<Client, String> {
    validate_addresses(&target.addresses)?;
    let mut builder = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(Duration::from_secs(2))
        // Preserve Host and TLS SNI while preventing a second DNS lookup from
        // changing a previously checked private address to a public address.
        .resolve_to_addrs(&target.host, &target.addresses);
    if let Some(ip) = interface_ip {
        builder = builder.local_address(IpAddr::V4(ip));
    }
    builder.build().map_err(|_| "无法创建音响网络连接。".into())
}

async fn request_pinned(
    target: Target,
    interface_ip: Option<Ipv4Addr>,
    method: Method,
    soap_action: Option<&str>,
    body: Option<String>,
) -> Result<(u16, String), String> {
    let client = pinned_client(&target, interface_ip)?;
    let mut request = client
        .request(method, target.url)
        .header("Accept", "text/xml")
        .header("Connection", "close");
    if let Some(action) = soap_action {
        request = request
            .header("SOAPAction", action)
            .header("Content-Type", "text/xml; charset=\"utf-8\"");
    }
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "连接音响超时，请确认音响在线且连接同一局域网。".to_string()
        } else {
            format!(
                "无法连接音响 {}，请检查网络、设备地址与局域网权限。",
                target.host
            )
        }
    })?;
    let status = response.status();
    if status.is_redirection() {
        return Err("设备地址发生重定向，请使用音响真实的局域网描述地址。".into());
    }
    if response
        .content_length()
        .is_some_and(|size| size > XML_LIMIT as u64)
    {
        return Err("音响返回的 XML 超过 1 MiB 限制。".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "读取音响响应失败或超时。".to_string())?;
        if bytes.len().saturating_add(chunk.len()) > XML_LIMIT {
            return Err("音响返回的 XML 超过 1 MiB 限制。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let text =
        String::from_utf8(bytes).map_err(|_| "音响返回的 XML 不是有效的 UTF-8。".to_string())?;
    Ok((status.as_u16(), text))
}

async fn request_xml(
    url: &str,
    interface_ip: Option<Ipv4Addr>,
    method: Method,
    soap_action: Option<&str>,
    body: Option<String>,
) -> Result<(u16, String), String> {
    // The outer deadline includes DNS, response headers and a slow body.
    time::timeout(REQUEST_TIMEOUT, async {
        let target = resolve_target(url).await?;
        request_pinned(target, interface_ip, method, soap_action, body).await
    })
    .await
    .map_err(|_| "连接音响超时，请确认设备在线且连接同一局域网。".to_string())?
}

fn parse_xml(text: &str) -> Result<Document<'_>, String> {
    if text.len() > XML_LIMIT {
        return Err("音响返回的 XML 超过 1 MiB 限制。".into());
    }
    // roxmltree rejects entity-bearing DTDs; also reject empty declarations.
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err("音响返回的 XML 含有不允许的 DTD 或实体声明。".into());
    }
    Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: XML_NODES_LIMIT,
        },
    )
    .map_err(|_| "音响返回了无法解析的 XML。".into())
}

fn child_text(node: Node<'_, '_>, name: &str) -> String {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
        .and_then(|child| child.text())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn service_version(service: &str, kind: &str) -> Option<u32> {
    service
        .strip_prefix(&format!("urn:schemas-upnp-org:service:{kind}:"))?
        .parse::<u32>()
        .ok()
        .filter(|version| *version > 0)
}

fn has_pro_word(value: &str) -> bool {
    value
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == "pro")
}

fn identify_xiaomi(name: &str, model: &str, manufacturer: &str) -> (bool, bool) {
    let identity = format!("{name} {model} {manufacturer}").to_lowercase();
    let is_xiaomi = ["xiaomi", "xiaoai", "xiao ai", "小米", "小爱"]
        .iter()
        .any(|marker| identity.contains(marker));
    let product = format!("{name} {model}").to_lowercase();
    let is_speaker = ["speaker", "音箱", "音响", "小爱", "xiaoai", "xiao ai"]
        .iter()
        .any(|marker| product.contains(marker));
    // Unknown model codes are deliberately not assigned to a product family.
    let is_pro = is_xiaomi && is_speaker && (has_pro_word(name) || has_pro_word(model));
    (is_xiaomi, is_pro)
}

fn parse_description(
    text: &str,
    location: &str,
    interface_ip: Option<Ipv4Addr>,
) -> Result<Vec<Device>, String> {
    let document = parse_xml(text)?;
    let location_url = parse_http_url(location)?;
    let root = document.root_element();
    let url_base = child_text(root, "URLBase");
    let base = if url_base.is_empty() {
        location_url.clone()
    } else {
        location_url
            .join(&url_base)
            .map_err(|_| "音响的 URLBase 地址无效。".to_string())?
    };
    let mut devices = Vec::new();
    for (index, node) in root
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "device")
        .enumerate()
    {
        let mut transport: Option<(u32, String, String)> = None;
        let mut rendering: Option<(u32, String, String)> = None;
        // Never borrow an AVTransport or RenderingControl from a sibling or
        // nested device: each embedded renderer has its own service pairing.
        for list in node
            .children()
            .filter(|child| child.is_element() && child.tag_name().name() == "serviceList")
        {
            for service in list
                .children()
                .filter(|child| child.is_element() && child.tag_name().name() == "service")
            {
                let kind = child_text(service, "serviceType");
                let control = child_text(service, "controlURL");
                if control.is_empty() {
                    continue;
                }
                let slot = if let Some(version) = service_version(&kind, "AVTransport") {
                    Some((&mut transport, version))
                } else {
                    service_version(&kind, "RenderingControl")
                        .map(|version| (&mut rendering, version))
                };
                if let Some((slot, version)) = slot {
                    if slot
                        .as_ref()
                        .is_none_or(|(current, _, _)| version > *current)
                    {
                        let control_url = base
                            .join(&control)
                            .map_err(|_| "音响服务控制地址无效。".to_string())?;
                        parse_http_url(control_url.as_str())?;
                        *slot = Some((version, control_url.to_string(), kind));
                    }
                }
            }
        }
        let Some((_, av_transport_url, av_transport_type)) = transport else {
            continue;
        };
        let (_, rendering_control_url, rendering_control_type) = rendering.unwrap_or_default();
        let model = child_text(node, "modelName");
        let mut name = child_text(node, "friendlyName");
        if name.is_empty() {
            name = if model.is_empty() {
                "DLNA 音响".into()
            } else {
                model.clone()
            };
        }
        let manufacturer = node
            .ancestors()
            .filter(|ancestor| ancestor.is_element() && ancestor.tag_name().name() == "device")
            .map(|ancestor| child_text(ancestor, "manufacturer"))
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        let (is_xiaomi, is_xiaomi_pro) = identify_xiaomi(&name, &model, &manufacturer);
        let id = child_text(node, "UDN");
        let address = host_string(&parse_http_url(&av_transport_url)?);
        devices.push(Device {
            id: if id.is_empty() {
                format!("{location}#device-{index}")
            } else {
                id
            },
            name,
            model,
            manufacturer,
            location: location_url.to_string(),
            address,
            av_transport_url,
            av_transport_type,
            rendering_control_url,
            rendering_control_type,
            is_xiaomi,
            is_xiaomi_pro,
            interface_ip: interface_ip.map(|ip| ip.to_string()),
        });
    }
    Ok(devices)
}

async fn fetch_devices(
    location: &str,
    interface_ip: Option<Ipv4Addr>,
) -> Result<Vec<Device>, String> {
    let (status, text) = request_xml(location, interface_ip, Method::GET, None, None).await?;
    if status != 200 {
        return Err(format!("获取音响设备描述失败：HTTP {status}。"));
    }
    let devices = parse_description(&text, location, interface_ip)?;
    for device in &devices {
        resolve_target(&device.av_transport_url).await?;
        if !device.rendering_control_url.is_empty() {
            resolve_target(&device.rendering_control_url).await?;
        }
    }
    Ok(devices)
}

fn sort_devices(devices: &mut [Device]) {
    devices.sort_by(|left, right| {
        right
            .is_xiaomi_pro
            .cmp(&left.is_xiaomi_pro)
            .then_with(|| right.is_xiaomi.cmp(&left.is_xiaomi))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub async fn fetch_device(location: &str) -> Result<Device, String> {
    let mut devices = time::timeout(DESCRIPTION_WINDOW, fetch_devices(location, None))
        .await
        .map_err(|_| "读取音响描述超时，请检查设备是否在线。".to_string())??;
    sort_devices(&mut devices);
    devices
        .into_iter()
        .next()
        .ok_or_else(|| "该设备未提供 DLNA AVTransport 服务，请确认音箱型号和固件支持 DLNA。".into())
}

fn usable_interface(ip: Ipv4Addr, is_up: bool) -> bool {
    is_up && !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast()
}

fn network_interfaces() -> Result<Vec<NetworkInterface>, String> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for interface in
        if_addrs::get_if_addrs().map_err(|error| format!("无法读取本机网卡：{error}"))?
    {
        if let IpAddr::V4(ip) = interface.ip() {
            if usable_interface(ip, interface.is_oper_up())
                && seen.insert((interface.name.clone(), ip))
            {
                found.push(NetworkInterface {
                    name: interface.name,
                    ip,
                });
            }
        }
    }
    found.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.ip.cmp(&right.ip))
    });
    Ok(found)
}

fn discovery_socket(interface: Ipv4Addr) -> Result<UdpSocket, String> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|error| format!("无法创建发现连接：{error}"))?;
    socket
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    socket
        .bind(&SocketAddr::new(IpAddr::V4(interface), 0).into())
        .map_err(|error| error.to_string())?;
    // Binding a source IP alone does not select multicast egress on every OS.
    socket
        .set_multicast_if_v4(&interface)
        .map_err(|error| error.to_string())?;
    socket
        .set_multicast_ttl_v4(2)
        .map_err(|error| error.to_string())?;
    let socket: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(socket).map_err(|error| error.to_string())
}

fn search_message(target: &str) -> String {
    format!("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n")
}

fn parse_ssdp_response(
    packet: &[u8],
    sender: SocketAddr,
    interface_ip: Ipv4Addr,
) -> Option<Candidate> {
    if packet.len() > 16 * 1024 || !is_lan_address(sender.ip()) {
        return None;
    }
    let text = std::str::from_utf8(packet).ok()?;
    let mut lines = text.lines();
    let first: Vec<_> = lines.next()?.split_whitespace().collect();
    if first.len() < 2 || !matches!(first[0], "HTTP/1.1" | "HTTP/1.0") || first[1] != "200" {
        return None;
    }
    let mut location = None;
    for line in lines {
        if line.trim().is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("location") {
            let normalized = parse_http_url(value.trim()).ok()?.to_string();
            if location
                .as_ref()
                .is_some_and(|previous| previous != &normalized)
            {
                return None;
            }
            location = Some(normalized);
        }
    }
    Some(Candidate {
        location: location?,
        interface_ip,
    })
}

async fn send_search_round(
    socket: &UdpSocket,
    destination: SocketAddr,
    deadline: Instant,
) -> Result<(), String> {
    for target in SSDP_TARGETS {
        time::timeout_at(
            deadline,
            socket.send_to(search_message(target).as_bytes(), destination),
        )
        .await
        .map_err(|_| "发现音响请求超时。".to_string())?
        .map_err(|error| format!("无法发送多播发现请求：{error}"))?;
    }
    Ok(())
}

async fn scan_interface(
    interface: NetworkInterface,
    destination: SocketAddr,
    window: Duration,
    retry_after: Duration,
) -> ScanResult {
    let mut result = ScanResult::default();
    let label = format!("{} ({})", interface.name, interface.ip);
    let socket = match discovery_socket(interface.ip) {
        Ok(socket) => socket,
        Err(error) => {
            result
                .warnings
                .push(format!("网卡 {label} 无法搜索音响：{error}"));
            return result;
        }
    };
    let deadline = Instant::now() + window;
    let retry_at = Instant::now() + retry_after;
    if let Err(error) = send_search_round(&socket, destination, deadline).await {
        result.warnings.push(format!("网卡 {label}：{error}"));
        return result;
    }
    let mut retried = false;
    let mut seen = HashSet::new();
    let mut buffer = vec![0_u8; 16 * 1024 + 1];
    loop {
        tokio::select! {
            _ = time::sleep_until(deadline) => break,
            _ = time::sleep_until(retry_at), if !retried => {
                retried = true;
                if let Err(error) = send_search_round(&socket, destination, deadline).await {
                    result.warnings.push(format!("网卡 {label} 重试失败：{error}"));
                }
            }
            received = socket.recv_from(&mut buffer) => {
                match received {
                    Ok((size, sender)) => {
                        if let Some(candidate) = parse_ssdp_response(&buffer[..size], sender, interface.ip) {
                            if seen.insert(candidate.location.clone()) {
                                result.candidates.push(candidate);
                                if result.candidates.len() >= MAX_LOCATIONS_PER_INTERFACE {
                                    result.warnings.push(format!("网卡 {label} 的设备响应较多，本轮已达到扫描上限。"));
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        result.warnings.push(format!("网卡 {label} 接收音响响应失败：{error}"));
                        break;
                    }
                }
            }
        }
    }
    result
}

pub async fn discover() -> Result<DiscoveryReport, String> {
    let interfaces = tokio::task::spawn_blocking(network_interfaces)
        .await
        .map_err(|_| "读取网卡信息失败。".to_string())??;
    let mut report = DiscoveryReport {
        interfaces: interfaces
            .iter()
            .map(|interface| format!("{} ({})", interface.name, interface.ip))
            .collect(),
        ..DiscoveryReport::default()
    };
    if interfaces.is_empty() {
        report
            .warnings
            .push("未找到活动的 IPv4 网卡，请连接音箱所在的 Wi-Fi 或有线网络。".into());
        return Ok(report);
    }
    let destination = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(239, 255, 255, 250)), 1900);
    let concurrency = interfaces.len();
    let scans = stream::iter(interfaces)
        .map(|interface| scan_interface(interface, destination, SEARCH_WINDOW, SEARCH_RETRY))
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for scan in scans {
        report.warnings.extend(scan.warnings);
        for candidate in scan.candidates {
            // Keep distinct routes until an HTTP description has succeeded.
            if seen.insert((candidate.location.clone(), candidate.interface_ip)) {
                candidates.push(candidate);
            }
        }
    }
    if candidates.len() > MAX_LOCATIONS {
        candidates.truncate(MAX_LOCATIONS);
        report
            .warnings
            .push("局域网设备较多，本轮已达到描述读取上限。".into());
    }
    let deadline = Instant::now() + DESCRIPTION_WINDOW;
    let mut reads = stream::iter(candidates)
        .map(|candidate| async move {
            time::timeout_at(
                deadline,
                fetch_devices(&candidate.location, Some(candidate.interface_ip)),
            )
            .await
            .map_err(|_| "读取设备描述超时。".to_string())?
        })
        .buffer_unordered(8);
    let mut failed = 0_usize;
    let mut by_id = HashMap::new();
    while let Some(result) = reads.next().await {
        match result {
            Ok(devices) => {
                for device in devices {
                    by_id
                        .entry(device.id.to_ascii_lowercase())
                        .or_insert(device);
                }
            }
            Err(_) => failed += 1,
        }
    }
    if failed > 0 {
        report.warnings.push(format!(
            "有 {failed} 个设备描述未能读取；设备可能已离线，或网络阻止了连接。"
        ));
    }
    report.devices = by_id.into_values().collect();
    sort_devices(&mut report.devices);
    Ok(report)
}

fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn didl_metadata(url: &str, title: &str, mime: &str) -> String {
    format!("<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\"><item id=\"0\" parentID=\"-1\" restricted=\"1\"><dc:title>{}</dc:title><upnp:class>object.item.audioItem.musicTrack</upnp:class><res protocolInfo=\"http-get:*:{}:*\">{}</res></item></DIDL-Lite>", xml_escape(title), xml_escape(mime), xml_escape(url))
}

fn parse_time(value: &str) -> Option<u64> {
    let value = value.trim();
    let (whole, fraction) = value
        .split_once('.')
        .map_or((value, None), |(whole, fraction)| (whole, Some(fraction)));
    if let Some(fraction) = fraction {
        let mut parts = fraction.split('/');
        let numerator = parts.next()?;
        if numerator.is_empty()
            || !numerator
                .bytes()
                .all(|character| character.is_ascii_digit())
        {
            return None;
        }
        if let Some(denominator) = parts.next() {
            if denominator.is_empty()
                || !denominator
                    .bytes()
                    .all(|character| character.is_ascii_digit())
                || denominator.parse::<u64>().ok()? == 0
                || parts.next().is_some()
            {
                return None;
            }
        }
    }
    let parts: Vec<_> = whole.split(':').collect();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty() || !part.bytes().all(|character| character.is_ascii_digit())
        })
    {
        return None;
    }
    let (hours, minutes, seconds) = (
        parts[0].parse::<u64>().ok()?,
        parts[1].parse::<u64>().ok()?,
        parts[2].parse::<u64>().ok()?,
    );
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    hours.checked_mul(3600)?.checked_add(minutes * 60 + seconds)
}

#[derive(Debug)]
struct SoapFailure {
    message: String,
    code: Option<u16>,
}

impl From<String> for SoapFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            code: None,
        }
    }
}

fn soap_values(
    action: &str,
    status: u16,
    text: &str,
) -> Result<HashMap<String, String>, SoapFailure> {
    if text.trim().is_empty() {
        if (200..300).contains(&status) {
            return Ok(HashMap::new());
        }
        return Err(format!("音响执行 {action} 失败：HTTP {status}。").into());
    }
    let document = parse_xml(text).map_err(SoapFailure::from)?;
    if let Some(fault) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Fault")
    {
        let value = |name: &str| {
            fault
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == name)
                .and_then(|node| node.text())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let code = value("errorCode").parse::<u16>().ok();
        let mut reason = value("errorDescription");
        if reason.is_empty() {
            reason = value("faultstring");
        }
        if reason.is_empty() {
            reason = "设备未说明原因".into();
        }
        let reason: String = reason.chars().take(512).collect();
        return Err(SoapFailure {
            message: format!(
                "音响执行 {action} 失败（{}）：{reason}",
                code.map(|code| code.to_string())
                    .unwrap_or_else(|| "UPnP".into())
            ),
            code,
        });
    }
    if !(200..300).contains(&status) {
        return Err(format!("音响执行 {action} 失败：HTTP {status}。").into());
    }
    let response_name = format!("{action}Response");
    let response = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == response_name)
        .ok_or_else(|| SoapFailure::from(format!("音响未返回有效的 {action} 响应。")))?;
    Ok(response
        .children()
        .filter(|child| child.is_element())
        .map(|child| {
            (
                child.tag_name().name().to_string(),
                child.text().unwrap_or("").trim().to_string(),
            )
        })
        .collect())
}

pub struct Controller {
    device: Device,
}

impl Controller {
    pub fn new(device: Device) -> Self {
        Self { device }
    }

    async fn soap(
        &self,
        action: &str,
        arguments: &[(&str, String)],
        rendering: bool,
    ) -> Result<HashMap<String, String>, SoapFailure> {
        let (url, service) = if rendering {
            (
                &self.device.rendering_control_url,
                &self.device.rendering_control_type,
            )
        } else {
            (
                &self.device.av_transport_url,
                &self.device.av_transport_type,
            )
        };
        if url.is_empty() || service.is_empty() {
            return Err(if rendering {
                "此音响没有提供音量控制服务。"
            } else {
                "此音响没有提供播放控制服务。"
            }
            .to_string()
            .into());
        }
        let mut args = "<InstanceID>0</InstanceID>".to_string();
        for (name, value) in arguments {
            args.push_str(&format!("<{name}>{}</{name}>", xml_escape(value)));
        }
        let body = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{args}</u:{action}></s:Body></s:Envelope>", xml_escape(service));
        let soap_action = format!("\"{service}#{action}\"");
        let interface = self
            .device
            .interface_ip
            .as_deref()
            .and_then(|ip| ip.parse().ok());
        let (status, response) =
            request_xml(url, interface, Method::POST, Some(&soap_action), Some(body)).await?;
        soap_values(action, status, &response)
    }

    pub async fn play(&self, url: &str, title: &str, mime: &str) -> Result<(), String> {
        resolve_target(url).await?;
        if title.chars().any(|character| {
            (character < '\u{20}' && !matches!(character, '\t' | '\r' | '\n'))
                || matches!(character, '\u{fffe}' | '\u{ffff}')
        }) {
            return Err("歌曲标题包含无法发送给音响的控制字符。".into());
        }
        let valid_mime = mime.strip_prefix("audio/").is_some_and(|subtype| {
            !subtype.is_empty()
                && subtype
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".+-_".contains(c))
        }) || mime == "application/ogg";
        if !valid_mime {
            return Err("音乐文件的音频类型无效。".into());
        }
        // Some renderers reject URI changes while playing. Idle/unsupported
        // Stop faults are harmless; connection failures must remain visible.
        if let Err(error) = self.soap("Stop", &[], false).await {
            if !matches!(error.code, Some(401 | 701)) {
                return Err(error.message);
            }
        }
        self.soap(
            "SetAVTransportURI",
            &[
                ("CurrentURI", url.into()),
                ("CurrentURIMetaData", didl_metadata(url, title, mime)),
            ],
            false,
        )
        .await
        .map_err(|error| error.message)?;
        self.resume().await
    }

    pub async fn pause(&self) -> Result<(), String> {
        self.soap("Pause", &[], false)
            .await
            .map(|_| ())
            .map_err(|error| error.message)
    }

    pub async fn resume(&self) -> Result<(), String> {
        self.soap("Play", &[("Speed", "1".into())], false)
            .await
            .map(|_| ())
            .map_err(|error| error.message)
    }

    pub async fn stop(&self) -> Result<(), String> {
        self.soap("Stop", &[], false)
            .await
            .map(|_| ())
            .map_err(|error| error.message)
    }

    pub async fn seek(&self, seconds: u64) -> Result<(), String> {
        let target = format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        );
        self.soap(
            "Seek",
            &[("Unit", "REL_TIME".into()), ("Target", target)],
            false,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.message)
    }

    pub async fn set_volume(&self, volume: u8) -> Result<(), String> {
        if volume > 100 {
            return Err("音量必须在 0 到 100 之间。".into());
        }
        self.soap(
            "SetVolume",
            &[
                ("Channel", "Master".into()),
                ("DesiredVolume", volume.to_string()),
            ],
            true,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.message)
    }

    pub async fn get_status(&self) -> Result<TransportStatus, String> {
        let transport = self
            .soap("GetTransportInfo", &[], false)
            .await
            .map_err(|error| error.message)?;
        let state = transport
            .get("CurrentTransportState")
            .map(|state| state.to_uppercase())
            .unwrap_or_default();
        if !matches!(
            state.as_str(),
            "STOPPED"
                | "PLAYING"
                | "PAUSED_PLAYBACK"
                | "TRANSITIONING"
                | "NO_MEDIA_PRESENT"
                | "RECORDING"
                | "PAUSED_RECORDING"
        ) {
            return Err("音响未提供有效的播放状态，请稍后重试。".into());
        }
        let mut status = TransportStatus {
            transport_state: state,
            position: 0,
            duration: 0,
            position_valid: false,
            volume: None,
            current_uri: None,
            warnings: Vec::new(),
        };
        match self.soap("GetPositionInfo", &[], false).await {
            Ok(values) => {
                let position = values.get("RelTime").and_then(|value| parse_time(value));
                let duration = values
                    .get("TrackDuration")
                    .and_then(|value| parse_time(value));
                status.position = position.unwrap_or(0);
                status.duration = duration.unwrap_or(0);
                status.position_valid = position.is_some() && duration.is_some();
                if !status.position_valid {
                    status
                        .warnings
                        .push("音响未提供有效的播放时间，本次状态不用于自动切歌。".into());
                }
                status.current_uri = values
                    .get("TrackURI")
                    .filter(|value| !value.is_empty() && *value != "NOT_IMPLEMENTED")
                    .cloned();
            }
            Err(error) => status.warnings.push(error.message),
        }
        if !self.device.rendering_control_url.is_empty() {
            match self
                .soap("GetVolume", &[("Channel", "Master".into())], true)
                .await
            {
                Ok(values) => {
                    status.volume = values
                        .get("CurrentVolume")
                        .and_then(|value| value.parse::<u8>().ok())
                        .filter(|volume| *volume <= 100);
                    if status.volume.is_none() {
                        status.warnings.push("音响未提供有效音量。".into());
                    }
                }
                Err(error) => status.warnings.push(error.message),
            }
        }
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::task::JoinHandle;

    const AVT: &str = "urn:schemas-upnp-org:service:AVTransport:1";
    const RCS: &str = "urn:schemas-upnp-org:service:RenderingControl:1";

    // Representative UPnP description fixture, not a captured hardware claim.
    // Product identification uses the explicit name/model, never model codes.
    fn pro_description(base: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
 <URLBase>{base}/base/</URLBase>
 <device><deviceType>urn:schemas-upnp-org:device:Basic:1</deviceType>
  <friendlyName>Gateway</friendlyName><UDN>uuid:gateway</UDN><manufacturer>Xiaomi</manufacturer>
  <serviceList><service><serviceType>{RCS}</serviceType><controlURL>/wrong-parent-volume</controlURL></service></serviceList>
  <deviceList><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
   <friendlyName>客厅 · 小爱音箱 Pro</friendlyName><modelName>XiaoAI Speaker Pro</modelName>
   <manufacturer>Xiaomi</manufacturer><UDN>uuid:xiaomi-pro-fixture</UDN>
   <serviceList>
    <service><serviceType>{AVT}</serviceType><controlURL>transport</controlURL></service>
    <service><serviceType>{RCS}</serviceType><controlURL>/rendering</controlURL></service>
   </serviceList>
  </device></deviceList>
 </device>
</root>"#
        )
    }

    #[derive(Default)]
    struct MockState {
        description: String,
        actions: Vec<(String, String, String)>,
        requests: Vec<(String, String)>,
        fail_actions: HashMap<String, u16>,
        disconnect: HashSet<String>,
        position_response: Option<String>,
        transport_response: Option<String>,
    }

    struct MockRenderer {
        base: String,
        state: Arc<Mutex<MockState>>,
        worker: JoinHandle<()>,
    }

    impl Drop for MockRenderer {
        fn drop(&mut self) {
            self.worker.abort();
        }
    }

    impl MockRenderer {
        async fn start() -> Self {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let state = Arc::new(Mutex::new(MockState {
                description: pro_description(&base),
                ..Default::default()
            }));
            let worker_state = state.clone();
            let worker = tokio::spawn(async move {
                while let Ok((connection, _)) = listener.accept().await {
                    let state = worker_state.clone();
                    tokio::spawn(async move {
                        serve_request(connection, state).await;
                    });
                }
            });
            Self {
                base,
                state,
                worker,
            }
        }

        async fn device(&self) -> Device {
            fetch_device(&format!("{}/description.xml", self.base))
                .await
                .unwrap()
        }
    }

    async fn serve_request(mut connection: TcpStream, state: Arc<Mutex<MockState>>) {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        let header_end = loop {
            let read = connection.read(&mut buffer).await.unwrap();
            if read == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(position) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                break position + 4;
            }
            assert!(
                request.len() < 32 * 1024,
                "test request headers exceeded limit"
            );
        };
        let headers = String::from_utf8(request[..header_end].to_vec()).unwrap();
        let path = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();
        let header = |key: &str| {
            headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.trim().to_string())
                .unwrap_or_default()
        };
        let length = header("Content-Length").parse::<usize>().unwrap_or(0);
        while request.len() < header_end + length {
            let read = connection.read(&mut buffer).await.unwrap();
            if read == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let body = String::from_utf8(request[header_end..header_end + length].to_vec()).unwrap();
        let action_header = header("SOAPAction");
        let action = action_header
            .trim_matches('"')
            .rsplit('#')
            .next()
            .unwrap_or("")
            .to_string();
        let (status, extra_headers, output) = {
            let mut state = state.lock().unwrap();
            state.requests.push((path.clone(), header("Host")));
            if headers.starts_with("GET ") {
                match path.as_str() {
                    "/redirect.xml" => (
                        302,
                        "Location: http://8.8.8.8/device.xml\r\n".to_string(),
                        String::new(),
                    ),
                    "/huge.xml" => (
                        200,
                        format!("Content-Length: {}\r\n", XML_LIMIT + 1),
                        String::new(),
                    ),
                    "/entity.xml" => (
                        200,
                        String::new(),
                        "<!DOCTYPE root [<!ENTITY unsafe 'secret'>]><root>&unsafe;</root>".into(),
                    ),
                    _ => (200, String::new(), state.description.clone()),
                }
            } else {
                state.actions.push((action.clone(), path, body));
                if state.disconnect.contains(&action) {
                    return;
                }
                let fault = state.fail_actions.get(&action).copied();
                let xml = if let Some(code) = fault {
                    format!("<s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>{code}</errorCode><errorDescription>Transition not available</errorDescription></UPnPError></detail></s:Fault>")
                } else {
                    let arguments = match action.as_str() {
                        "GetTransportInfo" => state.transport_response.as_deref().unwrap_or("<u:CurrentTransportState>PLAYING</u:CurrentTransportState>"),
                        "GetPositionInfo" => state.position_response.as_deref().unwrap_or("<u:TrackDuration>01:02:03.4</u:TrackDuration><u:RelTime>00:01:23</u:RelTime><u:TrackURI>http://127.0.0.1/music.mp3?x=1&amp;y=2</u:TrackURI>"),
                        "GetVolume" => "<u:CurrentVolume>28</u:CurrentVolume>",
                        _ => "",
                    };
                    let service = if action.ends_with("Volume") { RCS } else { AVT };
                    format!("<u:{action}Response xmlns:u=\"{service}\">{arguments}</u:{action}Response>")
                };
                (if fault.is_some() { 500 } else { 200 }, String::new(), format!("<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>{xml}</s:Body></s:Envelope>"))
            }
        };
        let content_length = if extra_headers.contains("Content-Length:") {
            String::new()
        } else {
            format!("Content-Length: {}\r\n", output.len())
        };
        let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: text/xml; charset=utf-8\r\n{content_length}{extra_headers}Connection: close\r\n\r\n{output}");
        let _ = connection.write_all(response.as_bytes()).await;
    }

    fn action_arguments(xml: &str) -> HashMap<String, String> {
        let document = Document::parse(xml).unwrap();
        let body = document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "Body")
            .unwrap();
        let action = body.children().find(|node| node.is_element()).unwrap();
        action
            .children()
            .filter(|node| node.is_element())
            .map(|node| {
                (
                    node.tag_name().name().into(),
                    node.text().unwrap_or("").into(),
                )
            })
            .collect()
    }

    #[test]
    fn xiaomi_pro_fixture_pairs_only_embedded_renderer_services() {
        let devices = parse_description(
            &pro_description("http://192.168.1.8:49152"),
            "http://192.168.1.8:49152/device.xml",
            Some(Ipv4Addr::new(192, 168, 1, 9)),
        )
        .unwrap();
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.id, "uuid:xiaomi-pro-fixture");
        assert_eq!(device.name, "客厅 · 小爱音箱 Pro");
        assert!(device.is_xiaomi && device.is_xiaomi_pro);
        assert_eq!(
            device.av_transport_url,
            "http://192.168.1.8:49152/base/transport"
        );
        assert_eq!(
            device.rendering_control_url,
            "http://192.168.1.8:49152/rendering"
        );
        assert_eq!(device.interface_ip.as_deref(), Some("192.168.1.9"));
    }

    #[test]
    fn unknown_devices_remain_available_and_model_codes_are_not_guessed() {
        let mut xml = pro_description("http://192.168.1.8");
        xml = xml
            .replace("客厅 · 小爱音箱 Pro", "Living room")
            .replace("XiaoAI Speaker Pro", "UNVERIFIED-123")
            .replace("Xiaomi", "Other vendor");
        let devices = parse_description(&xml, "http://192.168.1.8/device.xml", None).unwrap();
        assert_eq!(devices.len(), 1);
        assert!(!devices[0].is_xiaomi);
        assert!(!devices[0].is_xiaomi_pro);
        assert_eq!(
            identify_xiaomi("Living room", "UNVERIFIED-123", "Xiaomi"),
            (true, false)
        );
        assert_eq!(
            identify_xiaomi("Xiaomi TV Pro", "Television", "Xiaomi"),
            (true, false)
        );
        assert_eq!(
            identify_xiaomi("Proxima Speaker", "Speaker", "Xiaomi"),
            (true, false)
        );
        assert_eq!(identify_xiaomi("小爱音箱Pro", "", ""), (true, true));
    }

    #[test]
    fn missing_urlbase_and_missing_volume_service_do_not_borrow_parent_service() {
        let xml = pro_description("http://192.168.1.8")
            .replace("<URLBase>http://192.168.1.8/base/</URLBase>", "")
            .replace(&format!("<service><serviceType>{RCS}</serviceType><controlURL>/rendering</controlURL></service>"), "");
        let devices =
            parse_description(&xml, "http://192.168.1.8/nested/device.xml", None).unwrap();
        assert_eq!(
            devices[0].av_transport_url,
            "http://192.168.1.8/nested/transport"
        );
        assert!(devices[0].rendering_control_url.is_empty());
    }

    #[test]
    fn private_address_policy_rejects_public_multicast_and_mixed_dns_answers() {
        for value in [
            "http://8.8.8.8/x",
            "http://239.255.255.250/x",
            "http://0.0.0.0/x",
            "http://192.0.2.1/x",
            "http://[::]/x",
            "http://[ff02::c]/x",
            "http://[::ffff:8.8.8.8]/x",
            "http://u:p@127.0.0.1/x",
            "http://127.0.0.1/x#secret",
            "file:///tmp/x",
        ] {
            assert!(parse_http_url(value).is_err(), "accepted {value}");
        }
        for value in [
            "http://127.0.0.1:8000/x",
            "https://192.168.1.2/x",
            "http://10.0.0.2/x",
            "http://172.16.0.1/x",
            "http://169.254.1.2/x",
            "http://[::1]/x",
            "http://[fd12::123]/x",
        ] {
            assert!(parse_http_url(value).is_ok(), "rejected {value}");
        }
        assert!(validate_addresses(&[
            "127.0.0.1:80".parse().unwrap(),
            "8.8.8.8:80".parse().unwrap()
        ])
        .is_err());
    }

    #[test]
    fn descriptions_reject_external_control_urls_and_xml_entities() {
        let xml = pro_description("http://192.168.1.8").replace(
            "<controlURL>transport</controlURL>",
            "<controlURL>http://8.8.8.8/control</controlURL>",
        );
        assert!(parse_description(&xml, "http://192.168.1.8/device.xml", None).is_err());
        assert!(parse_xml("<!DOCTYPE root><root/>").is_err());
        assert!(parse_xml("<!DOCTYPE root [<!ENTITY a 'x'>]><root>&a;</root>").is_err());
        assert!(parse_xml(&" ".repeat(XML_LIMIT + 1)).is_err());
    }

    #[test]
    fn ssdp_parser_validates_status_location_sender_and_route() {
        let source = "192.168.1.8:1900".parse().unwrap();
        let interface = Ipv4Addr::new(192, 168, 1, 9);
        let response = b"HTTP/1.1 200 OK\r\nlocation: http://192.168.1.8:49152/device.xml\r\nUSN: uuid:test::upnp:rootdevice\r\n\r\n";
        let candidate = parse_ssdp_response(response, source, interface).unwrap();
        assert_eq!(candidate.location, "http://192.168.1.8:49152/device.xml");
        assert_eq!(candidate.interface_ip, interface);
        assert!(
            parse_ssdp_response(response, "8.8.8.8:1900".parse().unwrap(), interface).is_none()
        );
        assert!(parse_ssdp_response(
            b"NOTIFY * HTTP/1.1\r\nLOCATION: http://192.168.1.8/x\r\n\r\n",
            source,
            interface
        )
        .is_none());
        assert!(parse_ssdp_response(
            b"HTTP/1.1 200 OK\r\n\r\nLOCATION: http://192.168.1.8/x",
            source,
            interface
        )
        .is_none());
        assert!(parse_ssdp_response(
            b"HTTP/1.1 200 OK\r\nLOCATION: http://8.8.8.8/x\r\n\r\n",
            source,
            interface
        )
        .is_none());
        assert!(parse_ssdp_response(b"HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.8/a\r\nLOCATION: http://192.168.1.8/b\r\n\r\n", source, interface).is_none());
    }

    #[test]
    fn interface_filter_keeps_every_active_unicast_ipv4_and_time_conversion_is_checked() {
        assert!(usable_interface(Ipv4Addr::new(192, 168, 1, 9), true));
        assert!(usable_interface(Ipv4Addr::new(10, 8, 0, 1), true));
        assert!(!usable_interface(Ipv4Addr::new(192, 168, 1, 9), false));
        assert!(!usable_interface(Ipv4Addr::LOCALHOST, true));
        assert!(!usable_interface(Ipv4Addr::UNSPECIFIED, true));
        assert_eq!(parse_time("01:02:03.400"), Some(3723));
        assert_eq!(parse_time("27:00:03"), Some(97203));
        assert_eq!(parse_time("0:00:05.2/10"), Some(5));
        assert_eq!(parse_time("00:00:00"), Some(0));
        for invalid in [
            "NOT_IMPLEMENTED",
            "00:99:03",
            "18446744073709551615:01:00",
            "00:00:01.bad",
            "00:00:01.1/0",
            "00:00:01.1/2/3",
        ] {
            assert_eq!(parse_time(invalid), None, "accepted {invalid}");
        }
    }

    #[tokio::test]
    async fn discovery_socket_uses_selected_route_retries_all_targets_and_deduplicates() {
        let responder = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let destination = responder.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut targets = Vec::new();
            let mut bytes = [0_u8; 2048];
            for _ in 0..6 {
                let (size, source) =
                    time::timeout(Duration::from_secs(2), responder.recv_from(&mut bytes))
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(source.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
                let request = std::str::from_utf8(&bytes[..size]).unwrap();
                targets.push(
                    request
                        .lines()
                        .find(|line| line.starts_with("ST: "))
                        .unwrap()
                        .to_string(),
                );
                responder
                    .send_to(
                        b"HTTP/1.1 200 OK\r\nLOCATION: http://127.0.0.1:12345/device.xml\r\n\r\n",
                        source,
                    )
                    .await
                    .unwrap();
            }
            targets
        });
        let result = scan_interface(
            NetworkInterface {
                name: "test".into(),
                ip: Ipv4Addr::LOCALHOST,
            },
            destination,
            Duration::from_millis(300),
            Duration::from_millis(50),
        )
        .await;
        let targets = task.await.unwrap();
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].interface_ip, Ipv4Addr::LOCALHOST);
        for target in SSDP_TARGETS {
            assert_eq!(
                targets
                    .iter()
                    .filter(|entry| **entry == format!("ST: {target}"))
                    .count(),
                2
            );
        }
    }

    #[tokio::test]
    async fn local_http_playback_round_trips_metadata_then_controls_and_status() {
        let renderer = MockRenderer::start().await;
        let controller = Controller::new(renderer.device().await);
        let uri = format!("{}/music/a%20b.mp3?x=1&y=2", renderer.base);
        let title = "音乐 <现场> & \"朋友\"";
        controller.play(&uri, title, "audio/mpeg").await.unwrap();
        {
            let state = renderer.state.lock().unwrap();
            assert_eq!(
                state
                    .actions
                    .iter()
                    .map(|action| action.0.as_str())
                    .collect::<Vec<_>>(),
                ["Stop", "SetAVTransportURI", "Play"]
            );
            assert_eq!(state.actions[1].1, "/base/transport");
            let arguments = action_arguments(&state.actions[1].2);
            assert_eq!(arguments["InstanceID"], "0");
            assert_eq!(arguments["CurrentURI"], uri);
            let metadata = Document::parse(&arguments["CurrentURIMetaData"]).unwrap();
            let item = metadata
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "item")
                .unwrap();
            assert_eq!(child_text(item, "title"), title);
            assert_eq!(
                child_text(item, "class"),
                "object.item.audioItem.musicTrack"
            );
            let resource = item
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "res")
                .unwrap();
            assert_eq!(resource.text(), Some(uri.as_str()));
            assert_eq!(
                resource.attribute("protocolInfo"),
                Some("http-get:*:audio/mpeg:*")
            );
        }
        controller.pause().await.unwrap();
        controller.resume().await.unwrap();
        controller.seek(3723).await.unwrap();
        controller.set_volume(42).await.unwrap();
        controller.stop().await.unwrap();
        let status = controller.get_status().await.unwrap();
        assert_eq!(status.transport_state, "PLAYING");
        assert_eq!(
            (status.position, status.duration, status.volume),
            (83, 3723, Some(28))
        );
        assert!(status.position_valid);
        assert_eq!(
            status.current_uri.as_deref(),
            Some("http://127.0.0.1/music.mp3?x=1&y=2")
        );
        assert!(status.warnings.is_empty());
        let state = renderer.state.lock().unwrap();
        let seek = state
            .actions
            .iter()
            .find(|action| action.0 == "Seek")
            .unwrap();
        assert_eq!(action_arguments(&seek.2)["Target"], "01:02:03");
        let volume = state
            .actions
            .iter()
            .find(|action| action.0 == "SetVolume")
            .unwrap();
        assert_eq!(volume.1, "/rendering");
        assert_eq!(action_arguments(&volume.2)["DesiredVolume"], "42");
    }

    #[tokio::test]
    async fn only_known_stop_faults_are_ignored_and_failed_uri_never_plays() {
        let renderer = MockRenderer::start().await;
        let controller = Controller::new(renderer.device().await);
        let uri = format!("{}/music.mp3", renderer.base);
        for code in [401, 701] {
            renderer
                .state
                .lock()
                .unwrap()
                .fail_actions
                .insert("Stop".into(), code);
            controller.play(&uri, "Music", "audio/mpeg").await.unwrap();
        }
        {
            let mut state = renderer.state.lock().unwrap();
            state.actions.clear();
            state.fail_actions.insert("Stop".into(), 501);
        }
        assert!(controller
            .play(&uri, "Music", "audio/mpeg")
            .await
            .unwrap_err()
            .contains("501"));
        assert_eq!(renderer.state.lock().unwrap().actions.len(), 1);
        {
            let mut state = renderer.state.lock().unwrap();
            state.actions.clear();
            state.fail_actions.clear();
            state.fail_actions.insert("SetAVTransportURI".into(), 714);
        }
        assert!(controller
            .play(&uri, "Music", "audio/mpeg")
            .await
            .unwrap_err()
            .contains("714"));
        assert!(!renderer
            .state
            .lock()
            .unwrap()
            .actions
            .iter()
            .any(|action| action.0 == "Play"));
    }

    #[tokio::test]
    async fn connectivity_errors_remain_errors_but_optional_status_actions_become_warnings() {
        let renderer = MockRenderer::start().await;
        let controller = Controller::new(renderer.device().await);
        renderer
            .state
            .lock()
            .unwrap()
            .disconnect
            .insert("Stop".into());
        assert!(controller
            .play(
                &format!("{}/music.mp3", renderer.base),
                "Music",
                "audio/mpeg"
            )
            .await
            .is_err());
        {
            let mut state = renderer.state.lock().unwrap();
            state.fail_actions.insert("GetPositionInfo".into(), 401);
            state.fail_actions.insert("GetVolume".into(), 401);
        }
        let status = controller.get_status().await.unwrap();
        assert_eq!(status.transport_state, "PLAYING");
        assert_eq!(status.volume, None);
        assert_eq!(status.position, 0);
        assert!(!status.position_valid);
        assert_eq!(status.warnings.len(), 2);
        renderer
            .state
            .lock()
            .unwrap()
            .fail_actions
            .insert("GetTransportInfo".into(), 501);
        assert!(controller.get_status().await.unwrap_err().contains("501"));
    }

    #[tokio::test]
    async fn local_http_redirect_oversized_xml_and_dtd_are_rejected() {
        let renderer = MockRenderer::start().await;
        for (path, expected) in [
            ("/redirect.xml", "重定向"),
            ("/huge.xml", "1 MiB"),
            ("/entity.xml", "DTD"),
        ] {
            let error = fetch_device(&format!("{}{path}", renderer.base))
                .await
                .unwrap_err();
            assert!(error.contains(expected), "{path}: {error}");
        }
    }

    #[tokio::test]
    async fn missing_or_invalid_position_disarms_completion_but_volume_failure_does_not() {
        let renderer = MockRenderer::start().await;
        let controller = Controller::new(renderer.device().await);
        renderer
            .state
            .lock()
            .unwrap()
            .fail_actions
            .insert("GetVolume".into(), 401);
        let valid = controller.get_status().await.unwrap();
        assert!(valid.position_valid);
        assert!(valid.volume.is_none());
        for response in [
            "<u:RelTime>00:00:00</u:RelTime>",
            "<u:RelTime>NOT_IMPLEMENTED</u:RelTime><u:TrackDuration>00:03:00</u:TrackDuration>",
            "<u:RelTime>00:00:00</u:RelTime><u:TrackDuration>bad</u:TrackDuration>",
        ] {
            renderer.state.lock().unwrap().position_response = Some(response.into());
            assert!(
                !controller.get_status().await.unwrap().position_valid,
                "accepted {response}"
            );
        }
        renderer.state.lock().unwrap().position_response = Some("<u:RelTime>00:00:00</u:RelTime><u:TrackDuration>00:00:00</u:TrackDuration><u:TrackURI>NOT_IMPLEMENTED</u:TrackURI>".into());
        let reset = controller.get_status().await.unwrap();
        assert!(reset.position_valid);
        assert!(reset.current_uri.is_none());
    }

    #[tokio::test]
    async fn absent_or_invalid_transport_state_is_a_hard_failure() {
        let renderer = MockRenderer::start().await;
        let controller = Controller::new(renderer.device().await);
        for response in [
            "",
            "<u:CurrentTransportState>UNKNOWN</u:CurrentTransportState>",
        ] {
            renderer.state.lock().unwrap().transport_response = Some(response.into());
            assert!(controller
                .get_status()
                .await
                .unwrap_err()
                .contains("有效的播放状态"));
        }
    }

    #[tokio::test]
    async fn pinned_client_uses_checked_address_and_preserves_original_host() {
        let renderer = MockRenderer::start().await;
        let local_url = Url::parse(&renderer.base).unwrap();
        let port = local_url.port().unwrap();
        let host = "speaker.invalid";
        let target = Target {
            url: Url::parse(&format!("http://{host}:{port}/description.xml")).unwrap(),
            host: host.into(),
            addresses: vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)],
        };
        let (status, _) =
            request_pinned(target, Some(Ipv4Addr::LOCALHOST), Method::GET, None, None)
                .await
                .unwrap();
        assert_eq!(status, 200);
        assert_eq!(
            renderer.state.lock().unwrap().requests[0].1,
            format!("{host}:{port}")
        );
    }
}
