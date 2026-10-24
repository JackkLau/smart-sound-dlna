// 国内直连、海外代理；在 FlClash 的配置扩展中绑定此脚本。
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
