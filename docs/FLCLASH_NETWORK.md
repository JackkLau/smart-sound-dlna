# FlClash 国内直连 / 海外代理配置与排障

适用于本机 macOS 上的 **FlClash 0.9.1**。核对日期：2026-10-23。本文按访问目标分流：**中国大陆服务直连，海外服务走代理，本机与局域网直连，未匹配流量默认走代理**。只需一套规则配置，并针对本项目的小米登录、Bilibili 下载和 DLNA 投屏给出检查方法。

## 1. 先改最影响调试的设置

本次只读检查得到以下记录；这些是本机安装信息、保存设置和生成配置，**不等同于已验证的实时连接状态**。

| 项目 | 当前记录 | 调试时建议 |
| --- | --- | --- |
| FlClash 版本 | 0.9.1 | 按本文对应版本操作 |
| 出站模式 | `global`，全局 | 改为 **规则**；全局模式不执行国内外分流规则 |
| 混合代理端口 | `7890` | 下文显式代理测试使用此端口；改过端口需同步替换 |
| 系统代理 | 保存设置为开启 | 浏览器调试可开启；原生请求另行检查 |
| 虚拟网卡 TUN | 生成配置为关闭 | 先保持关闭，验证原生直连；需要接管原生请求时再开启 |
| 日志级别 | `error` | 临时改为 **info**；信息不足时短暂用 debug |
| DNS | 腾讯 / 阿里 DoH，`fake-ip` | 现有配置未设置 `nameserver-policy`，不能据此确认 DNS 已按国内外分开 |
| IPv6 | 关闭 | 首轮排障维持关闭；恢复后单独复测 IPv6 |
| 局域网代理 | 开启，监听 `*` | 只有电脑自己使用代理时可关闭；它不是投屏所需开关 |
| 绕过私有网络 | 保存设置为关闭 | 开启 TUN 调试投屏前，打开此项并核对最终路由 |

**最快恢复可解释的状态：**

1. 在仪表盘把出站模式切到 **规则**。
2. 在代理页，将 `🇨🇳 国内网站`、`🌏 爱奇艺&哔哩哔哩` 选为 `DIRECT`。本机生成配置中存在这两个组；订阅更新后名称可能变化。
3. 在 `🔰 选择节点` 中固定一个可用节点。调试期间先不自动切换节点。
4. 在工具的相关配置入口将日志改为 `info`，打开日志捕获；查看请求 / 连接记录中的域名、规则、策略组和实际出口。
5. 保持 TUN 关闭，先验证小米登录、Bilibili 和投屏。再按第 6 节逐层检查。
6. 改完后结束相关旧连接或重启被测应用，再发起新请求。已有连接不会自动证明新规则已生效。

只做以上步骤即可先利用原订阅的分流。需要一套便于排查、明确区分国内外目标的配置，再使用第 4 节脚本。

## 2. 按访问目标决定出口

本文中「国内」指中国大陆服务。服务归类按域名与规则判断，不代表服务器物理位置或服务可用性。分流策略不因电脑所在地自动切换。

| 访问目标 | 出口 | 预期策略组 / 设置 |
| --- | --- | --- |
| 本机、路由器、小爱音箱、局域网音乐流 | 直连 | 本地绕过或 `DIRECT` |
| 小米账号 / 小爱云 API | 直连 | `大陆服务 → DIRECT` |
| Bilibili 网页、API、图片和音频 CDN | 直连 | `大陆服务 → DIRECT` |
| Google、GitHub、海外开发服务 | 选定的海外节点 | `海外服务 → 节点选择组 → 固定节点` |
| 其他匹配国内域名 / IP 规则的服务 | 直连 | `大陆服务 → DIRECT` |
| 未匹配流量 | 海外代理 | `其他流量 → 海外服务` |
| 解析代理节点地址的 DNS | 直连 | `proxy-server-nameserver` |
| 解析大陆服务的 DNS | 大陆 DNS，直连 | `nameserver-policy` |
| 解析海外服务的 DNS | 海外 DoH，经海外策略组 | `nameserver` 中的 `#海外服务` |

**规则顺序：本机 / 局域网直连 → 明确的业务域名 → 国内域名 → 国内 IP → 其余代理。** 需要临时调整单个服务时，在通用规则前增加它的准确域名，避免直接改成全局模式。

## 3. 为什么浏览器能用，项目仍可能失败

| 链路 | 本项目的行为 | FlClash 对它的影响 |
| --- | --- | --- |
| 官方登录网页 | 使用桌面 WebView | 可能受系统代理、系统 DNS 和 TUN 影响，需观察实际连接 |
| 小米云 API、Bilibili API / 下载 | Rust HTTP 客户端显式调用 `.no_proxy()` | 不使用 HTTP / SOCKS 显式代理；TUN、路由器代理和系统 DNS 仍可能影响它 |
| 音箱描述、SOAP 控制 | 局域网客户端显式禁用代理 | 需要本地路由正常；TUN 和其他 VPN 仍可能干扰 |
| SSDP 发现 | IPv4 UDP 组播 `239.255.255.250:1900` | 需要组播、正确网卡、同一局域网 |
| 音乐推流 | 音箱连接电脑的 HTTP 服务 | 需要电脑局域网地址、随机 TCP 端口和入站权限 |

依据：[原生在线客户端](../src-tauri/src/online.rs)、[小米云接口](../src-tauri/src/voice.rs)、[Bilibili 请求](../src-tauri/src/bilibili.rs)、[DLNA](../src-tauri/src/dlna.rs)、[媒体服务](../src-tauri/src/media.rs)。

因此，只设置 `HTTP_PROXY` / `HTTPS_PROXY` 或只开启系统代理，不能强制本项目的原生 HTTP 请求进入 FlClash。要让这些请求按规则走代理，可使用 TUN；验证时必须同时保证局域网绕过。

## 4. 可复制的国内外分流覆写脚本

### 4.1 使用范围

此脚本保留订阅提供的节点、代理集和已有策略组，新增三个清晰的策略组：`大陆服务`、`海外服务`、`其他流量`。为便于定位问题，**它会替换该配置的 `rules` 和 `dns`**，不保留原订阅的广告、游戏、流媒体等细分规则。这是一套调试基线；有这些需求时，先回原配置或在验证后逐项加入。

脚本不切换系统代理、TUN、日志级别、IPv6 或端口。这些设置由 FlClash 界面管理；尤其是出站模式，仍须手动选 **规则**。本机已存在 `GEOSITE.dat` 和 GeoIP 数据，脚本使用 `GEOSITE,cn` 和 `GEOIP,CN`，启用时仍须确认内核没有报告数据缺失或标签不存在。

### 4.2 在 FlClash 0.9.1 中配置

1. 先在 FlClash 备份入口导出当前设置，记录当前订阅、节点、出站模式、系统代理和 TUN 状态。备份保存在私人位置。
2. 保留原配置。另建一个配置条目，可再次导入同一订阅，命名为 **国内直连·海外代理**。
3. 打开 **工具 → 进阶配置 → 脚本 → ＋ → 导入文件**，选择同目录下的 [flclash-domestic-direct.js](flclash-domestic-direct.js)。导入后名称为 `flclash-domestic-direct`。也可新建脚本 `国内外分流`，粘贴下方内容。
4. 打开新配置的 **扩展** 页面，将方式切到 **脚本**，选择刚导入的 `flclash-domestic-direct`（或手动新建的 `国内外分流`）。仅导入 / 创建脚本还不会作用于配置。
5. 检查 **工具 → 进阶配置 → DNS** 里的全局覆写项。先记录原值，再移除会覆盖脚本 DNS 的项目；URL / 文件配置可能继续受这里的设置影响。也检查配置自己的覆写项。
6. 暂时停用与本次分流冲突的附加规则，尤其是提前匹配的 `MATCH`、进程代理和宽泛域名规则。FlClash 可将附加规则放在订阅规则前面。
7. 启用新配置，出站模式选 **规则**，核对 `大陆服务 → DIRECT`、`海外服务 → 🔰 选择节点 → 固定海外节点`、`其他流量 → 海外服务`。查看 **运行时配置** 确认 `rules`、`dns`、模式和组名都符合预期，再发起新连接。

脚本中的 `NODE_GROUP` 来自本机当前订阅的组名 `🔰 选择节点`。使用其他订阅时，改成其中实际存在的节点选择组，保持文字、空格与表情完全一致。

对应版本的入口与行为参见 [进阶配置](https://github.com/chen08209/FlClash/blob/v0.9.1/lib/views/config/advanced.dart)、[配置脚本绑定](https://github.com/chen08209/FlClash/blob/v0.9.1/lib/views/profiles/extend/script.dart)、[最终配置组装](https://github.com/chen08209/FlClash/blob/v0.9.1/lib/common/task.dart)。

### 4.3 脚本正文

```javascript
function main(config) {
  const NODE_GROUP = "🔰 选择节点";
  const groups = config["proxy-groups"] || [];
  if (!groups.some(group => group.name === NODE_GROUP)) {
    throw new Error("找不到节点组，请修改 NODE_GROUP：" + NODE_GROUP);
  }
  const owned = ["大陆服务", "海外服务", "其他流量"];
  config["proxy-groups"] = groups
    .filter(group => !owned.includes(group.name))
    .concat([
      { name: "大陆服务", type: "select", proxies: ["DIRECT"] },
      { name: "海外服务", type: "select",
        proxies: [NODE_GROUP] },
      { name: "其他流量", type: "select",
        proxies: ["海外服务"] }
    ]);

  const local4 = [
    "127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12",
    "192.168.0.0/16", "169.254.0.0/16", "224.0.0.0/4"
  ];
  const local6 = ["::1/128", "fc00::/7", "fe80::/10", "ff00::/8"];
  const chinaDomains = [
    "mi.com", "xiaomi.com", "miui.com", "xiaomi.net",
    "bilibili.com", "b23.tv", "bilivideo.com", "bilivideo.cn",
    "bilivideo.net", "hdslb.com", "biliapi.net", "biliapi.com"
  ];
  const overseasDomains = [
    "google.com", "googleapis.com", "gstatic.com",
    "github.com", "githubusercontent.com", "githubassets.com",
    "openai.com", "chatgpt.com", "oaistatic.com", "oaiusercontent.com",
    "anthropic.com", "claude.ai"
  ];
  config.rules = [
    "DOMAIN,localhost,DIRECT",
    "DOMAIN-SUFFIX,lan,DIRECT",
    "DOMAIN-SUFFIX,local,DIRECT",
    ...local4.map(cidr => "IP-CIDR," + cidr + ",DIRECT,no-resolve"),
    ...local6.map(cidr => "IP-CIDR6," + cidr + ",DIRECT,no-resolve"),
    ...chinaDomains.map(domain => "DOMAIN-SUFFIX," + domain + ",大陆服务"),
    ...overseasDomains.map(domain => "DOMAIN-SUFFIX," + domain + ",海外服务"),
    "GEOSITE,cn,大陆服务",
    "GEOIP,CN,大陆服务,no-resolve",
    "MATCH,其他流量"
  ];

  const localDns = [
    "https://dns.alidns.com/dns-query#DIRECT",
    "https://doh.pub/dns-query#DIRECT"
  ];
  const policy = { "geosite:cn": localDns };
  chinaDomains.forEach(domain => { policy["+." + domain] = localDns; });
  config.dns = {
    enable: true,
    ipv6: false,
    "enhanced-mode": "redir-host",
    "use-hosts": true,
    "use-system-hosts": true,
    "prefer-h3": false,
    "respect-rules": false,
    "default-nameserver": ["223.5.5.5", "119.29.29.29"],
    "proxy-server-nameserver": localDns,
    nameserver: [
      "https://cloudflare-dns.com/dns-query#海外服务",
      "https://dns.google/dns-query#海外服务"
    ],
    "nameserver-policy": policy
  };

  config.tun = config.tun || {};
  config.tun["route-exclude-address"] = Array.from(new Set([
    ...(config.tun["route-exclude-address"] || []), ...local4, ...local6
  ]));
  return config;
}
```

### 4.4 这套基线的几个选择

- **国内业务直连，海外业务与未匹配流量走代理。** 新增海外组没有直连备用项；仍须核对原节点选择组实际选中的是海外节点，不能选 `DIRECT` 或最终回退到直连的组。
- **固定一个节点再调试。** 先排除自动切换出口造成的差异，后续按连接记录调整具体域名规则。
- **先使用 `redir-host`。** 便于观察真实目标 IP，减少 Fake-IP 缓存对首轮排障的干扰。这是本文的调试选择，并非所有网络都优于 Fake-IP；稳定后可单独改回并复测。
- **DNS 用 `#DIRECT` / `#海外服务` 指定出口。** 所以这里 `respect-rules` 为 false，DNS 连接仍有明确出口。节点域名由直连 DNS 单独解析，避免「要连节点才能解析节点地址」的依赖环。
- **DNS 分流与流量分流分别设置。** `nameserver-policy` 决定谁解析；`rules` 决定请求从哪里出去。海外网站解析成功不代表节点能连接，DNS 所在地区也不等于业务出口地区。
- **规则自上而下匹配，`MATCH` 放最后。** 示例域名不是全部海外服务清单，未覆盖的大陆域名依赖 GeoSite / GeoIP，最终由 `其他流量` 接管。IP 地理位置也不能精确代表业务归属。

DNS 字段与出口参数依据 [Mihomo DNS 文档](https://wiki.metacubex.one/config/dns/)；匹配行为依据 [路由规则文档](https://wiki.metacubex.one/config/rules/)。

## 5. TUN 与投屏：同时保住本地网络

先用「TUN 关闭」建立投屏基线；只有需要接管原生请求，或要对照检查 TUN 影响时再开启。

在 **工具 → 进阶配置 → 网络** 的虚拟网卡相关设置中，启用 **绕过私有网络**，核对路由排除地址。上方脚本同时给出常用私网、回环、链路本地和组播网段；最终以运行时配置及系统实际路由为准。

`DIRECT` 规则是在流量进入内核后决定直连，`route-exclude-address` 是让对应目标绕过 TUN 路由。两者处在不同层。路由排除不会为已显式交给 HTTP 代理的请求自动改变目标规则。字段含义参见 [Mihomo TUN 文档](https://wiki.metacubex.one/config/inbound/tun/)。

投屏仍须逐项验证：

1. 电脑、音箱在同一可互访局域网，避免访客 Wi-Fi 和 AP 隔离。
2. 小爱音箱已开启 DLNA，电脑允许应用访问本地网络和接受入站连接。
3. 在应用诊断信息里核对媒体地址：应是音箱能访问的电脑局域网 IP。若显示 Fake-IP、VPN 地址或 `127.0.0.1`，先排查网卡选择和路由。
4. TUN 关闭可发现、开启后不可发现，优先查组播路由、排除网段和其他 VPN。不要只添加 `DST-PORT,1900,DIRECT`，SSDP 发现还依赖接口和组播。
5. 能发现但不能播放，检查音箱 → 电脑的随机 TCP 推流端口、入站权限，再用常见 MP3 排除编码兼容问题。FlClash 代理端口 `7890` 不是音乐服务端口。

`局域网代理 / allow-lan` 只控制其他设备能否连接电脑上的代理入口。小爱音箱回连本项目的媒体服务不需要开启它。

## 6. 固定排查顺序，每次只改变一层

### 第一步：记录当前状态

记下所在地、Wi-Fi / 有线、配置名、规则模式、固定节点、系统代理、TUN 和日志级别。复现时记下时间与具体功能，例如「读取 Bilibili 收藏夹」，不要只记「网络不通」。

### 第二步：用相同网址对比原生路径和显式代理路径

以下命令不带账号凭据，仅检查 DNS、连接、TLS 和 HTTP 响应。它们不会修改网络设置。示例适用于 macOS 终端；端口按实际设置替换。

```sh
# 不使用显式代理；要测试物理网络直连，应先关闭 TUN 和其他 VPN
curl --noproxy '*' --connect-timeout 8 --max-time 20 -sS -o /dev/null \
  -w 'HTTP=%{http_code} peer=%{remote_ip} DNS=%{time_namelookup}s TCP=%{time_connect}s TLS=%{time_appconnect}s total=%{time_total}s\n' \
  https://www.bilibili.com/

# 同一网址，显式交给 FlClash
curl --noproxy '' -x http://127.0.0.1:7890 --connect-timeout 8 --max-time 20 -sS -o /dev/null \
  -w 'HTTP=%{http_code} peer=%{remote_ip} total=%{time_total}s\n' \
  https://www.bilibili.com/

# 同样两条路径验证海外站点
curl --noproxy '*' --connect-timeout 8 --max-time 20 -I https://github.com/
curl --noproxy '' -x http://127.0.0.1:7890 --connect-timeout 8 --max-time 20 -I https://github.com/
```

如果 FlClash 开启了代理入口认证，第二种测试须按自己的认证设置执行；`407` 表示入口认证问题。不要把密码写入共享排障记录。

**`--noproxy '*'` 只禁用显式代理，不能绕过 TUN。** 显式代理测试也不能单独证明业务经节点出站：即使 FlClash 按 `DIRECT` 转发，curl 的 `peer` 仍可能显示本地代理地址。实际出口看 FlClash 连接记录。

### 第三步：看命中了哪条规则

| 测试 | 预期分流 | 核对重点 |
| --- | --- | --- |
| `account.xiaomi.com`、`api2.mina.mi.com` | `大陆服务 → DIRECT` | 登录页面和原生 API 分开检查 |
| `api.bilibili.com`、实际 `*.bilivideo.*` 下载域名 | `大陆服务 → DIRECT` | 网页、API 与 CDN 分开检查 |
| `github.com`、`*.githubusercontent.com` | `海外服务 → 固定节点` | 实际节点不能为 `DIRECT` |
| 音箱 IP、电脑局域网媒体 IP | 本地绕过或 `DIRECT` | 路由须走音箱可达的局域网接口 |

TUN 关闭时，本项目的 Rust 原生请求通常不会出现在 FlClash 连接列表；开启私网绕过后，投屏流量也可能不出现。这种缺失不应直接判定为请求未发出。

若 `www.bilibili.com` 能访问但下载不行，要查实际音频 CDN 域名。小米网页登录、设备列表和对话轮询也属于不同请求，逐个验证，不能互相代替。

### 第四步：需要时单独检查 DNS 和路由

```sh
# 查看 macOS 当前解析器；结果只反映系统设置，不一定是 FlClash 的完整解析过程
scutil --dns

# 查询系统解析结果
dscacheutil -q host -a name api.bilibili.com

# 把示例 IP 换成真实音箱地址，核对是否走正确的局域网接口
route -n get 192.168.1.50

# 查看 SSDP 组播目标的路由
route -n get 239.255.255.250
```

不要假定 Wi-Fi 永远是 `en0`。`dig @223.5.5.5 ...` 测的是指定 DNS 路径，在 TUN DNS 劫持开启时还可能被截获；它不能单独证明应用实际使用了这台服务器。

脚本默认不新增 DNS 监听端口。只有运行时配置确实启用了对应监听时，才能用 `dig @127.0.0.1 -p 端口 ...` 检查；不要直接假定 `1053` 可用。

### 第五步：按现象缩小范围

| 现象 | 优先检查 |
| --- | --- |
| 原生路径正常，显式代理失败 | FlClash 是否启动、端口 / 认证、命中规则、所选节点 |
| 原生路径失败，显式代理成功 | 直连 DNS / 网络与代理路径差异；原生客户端是否需要 TUN |
| 浏览器登录成功，应用账号连接失败 | 登录后的原生 API 路径、Cookie 会话、服务响应 |
| 大陆网站也走海外节点 | 当前是否仍为全局模式、前置附加规则、国内策略组选择 |
| DNS 超时 / `Could not resolve host` | 实际 DNS、DoH 可达性、节点域名引导解析、全局 DNS 覆写 |
| 只有开 TUN 后异常 | 路由排除、DNS 劫持、其他 VPN、网卡选择；再对照调整协议栈 |
| 搜不到音箱 | 同网段、DLNA、组播、AP 隔离、本地网络权限 |
| 找到音箱却播放失败 | 音箱回连电脑 IP / 随机 TCP 端口、入站权限、音频格式 |
| HTTP 401 / 403 | 已收到 HTTP 响应；继续查登录、权限、出口地区和服务限制 |
| HTTP 412 / 429 | 已收到 HTTP 响应；查请求限制 / 风控，避免循环高频重试 |
| `HTTP=000` 或 TLS 错误 | 尚无有效 HTTP 响应；查 DNS、连接、证书、系统时间和代理路径 |

HTTP 响应只能证明该次请求到了能返回响应的一端；不保证已登录、API 正确或业务可用。站点根路径、HEAD 与真实 API 的结果也可能不同。

## 7. 回滚与长期维护

1. 首选切回保留的原配置，恢复原来的策略组选择、DNS 覆写和附加规则。也可解除调试脚本绑定；不要删除订阅或清空应用数据。
2. 恢复记录过的系统代理、TUN、系统 DNS 管理开关和日志级别；它们可能是应用全局设置，单纯切换配置不一定恢复。
3. 若曾在 macOS 网络设置里手动改过 DNS，按原记录恢复。随后结束旧连接或重启被测应用，再验证国内、海外和局域网三条链路。
4. 日常保持规则模式；调试时固定节点，稳定后再加入自动选择、Fake-IP、IPv6 和更多业务规则，每加入一项都复测。
5. 本机生成配置位于 `~/Library/Application Support/com.follow.clash/config.yaml`。可只读核对；不要把它当成长期编辑入口，客户端可能重新生成。
6. 排障记录只需保留：复现时间、环境、功能 / 域名、命中规则、策略组、实际出口类型和错误。分享前删除订阅 URL、节点凭据、Cookie、`serviceToken`、签名下载 URL 和完整音乐推流链接。

## 8. 参考与验证边界

- [FlClash 官方项目](https://github.com/chen08209/FlClash)：确认项目身份，避免将其他 Clash 客户端的入口套用到本机。
- [0.9.1 中文界面文本](https://github.com/chen08209/FlClash/blob/v0.9.1/arb/intl_zh_CN.arb)：菜单、DNS 覆写、网络和运行时配置术语。
- [0.9.1 最终配置组装](https://github.com/chen08209/FlClash/blob/v0.9.1/lib/common/task.dart)：应用管理模式 / 端口、DNS 覆写、私网绕过和附加规则。
- [Mihomo DNS](https://wiki.metacubex.one/config/dns/)、[路由规则](https://wiki.metacubex.one/config/rules/)、[TUN](https://wiki.metacubex.one/config/inbound/tun/)：内核字段定义；较新字段需结合客户端所带内核确认。

脚本已执行离线检查：确认保留订阅节点与代理集、策略组及规则目标引用有效、国内组固定直连、海外及兜底组指向代理、节点 DNS 使用直连、重复执行不重复添加策略组 / 排除网段，以及缺失节点组时能明确报错。这些检查不等同于 Mihomo 实际加载或网络连通验证。

本次交付为配置文档与可复制示例，没有应用到正在使用的 FlClash，也没有执行真实音箱播放、账号登录或海外节点可用性测试。脚本应先在保留原配置的独立条目中启用，并通过运行时配置与真实请求确认。

## 补充：以后切换所在地时

当前方案按访问目标工作，电脑移到海外后，海外服务仍会走选定代理。若以后需要「人在海外时，海外服务改为直连」，可另外保存一个配置，再调整海外组与当地可达的 DNS；无需为当前需求创建第二套环境。

`DIRECT` 指从电脑当前网络直接出去，不会生成大陆出口。电脑身处海外但服务必须使用大陆出口时，需要另有真实的回大陆节点，并配套调整该业务的 DNS；这属于额外需求。
