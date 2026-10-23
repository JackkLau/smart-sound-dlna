# 小米音乐投送 · Rust + Tauri 2

Windows / macOS 桌面客户端，将电脑里的本地音乐通过 DLNA 投送到小米 / 小爱音箱。原生 Rust 负责设备发现、音乐库、HTTP 音频推流和播放控制，Tauri 2 提供桌面窗口与系统文件选择器。安装后的应用无需 Python、Node.js 或单独启动服务。

## 使用

1. 让电脑和音箱连接同一局域网，在小爱音箱 App 中开启 **我的 → DLNA 音乐播放**。小米官方列出的支持型号包含**小爱音箱 Pro**；具体入口随 App 版本可能变化。[小米官方说明](https://ics-resources.xiaomi.com/ics-resources/articles/6066c6ebe4801400d6db9c70.html)
2. 打开「小米音乐投送」，允许系统提示的本地网络访问。应用启动后自动搜索设备，随后约每 30 秒刷新，也可手动重新搜索。
3. 选择音箱，再用「添加音乐」或「导入文件夹」打开系统选择窗口。点击歌曲即可播放。

支持暂停、继续、停止、上一首 / 下一首、进度、音量、顺序播放、单曲循环、列表循环。播放期间电脑必须保持唤醒且应用保持运行；退出应用会关闭推流服务。

### 小爱音箱 Pro 自动发现

- 对每张活动的 IPv4 网卡发送 SSDP 查询，覆盖 `MediaRenderer`、`AVTransport` 和通用设备响应，并重复查询一次。
- 读取音箱自己的设备描述，以 UUID 去重，优先显示小爱音箱 Pro，再显示其他小米设备。
- Pro 标识依据设备上报的名称 / 型号与小米厂商信息判断。改名或某些固件隐藏型号时仍显示可用设备，但不会猜测 Pro 型号。
- 使用发现音箱的网卡提供音乐链接，减少有线、Wi-Fi、VPN 并存时的选错网卡问题。
- 不会自动播放或自动切换选中的设备。切换设备时会尝试停止本应用在原音箱上的播放。

自动发现能否在实际网络成功，仍取决于音箱固件、DLNA 开关和路由器组播设置。模拟设备通过不等同于真实 Pro 实机验证。

### 本地音乐

导入只索引原文件，不复制、不上传云端、不修改音乐。支持递归文件夹、中文与空格路径、重复导入去重、重启保留音乐库，最多 20,000 首。隐藏目录和符号链接 / Windows 重解析链接会跳过。源文件被移动、删除或替换后，请重新导入。

可导入 `MP3 FLAC WAV M4A AAC OGG OPUS WMA AIFF AIF`。目前直接传输原始文件，不内置转码；实际音频编码、采样率、声道兼容性由音箱决定。第一次连接建议用常见 MP3 验证。

## 小爱语音点歌

1. 先在主界面选择 DLNA 播放音响，然后打开左侧 **小爱语音**。
2. 点击 **登录小米账号**，在小米官方登录窗口完成登录，再回到客户端点击 **完成登录**。请选择绑定音响的账号。
3. 从账号设备列表选择接收语音的小爱，将它绑定到当前播放音响，点击 **开始接收指令**。
4. 说“小爱同学”，再说 **播放本地 + 歌名**，例如“播放本地晴天”。也支持“本地下一首 / 上一首 / 暂停 / 继续 / 停止 / 音量三十 / 单曲循环 / 列表循环 / 顺序播放”。

下方“测试指令”会通过同一指令处理层实际控制当前音响，不需要先登录账号，适合检查歌名匹配与 DLNA 控制。精确歌名优先；部分歌名只能匹配到一首时才会播放，多个匹配会提示补充歌名。切换主界面音响后，需更新语音绑定。

小爱并没有通用的 DLNA 语音点歌接口。本功能通过 **小米云端对话记录轮询**桥接到本地曲库，约每 3 秒读取一次，只执行启用后的新“本地”指令。语音识别仍由小爱完成，电脑不录音。普通聊天不执行、不保存；会话、语音绑定和最近一次指令结果仅保留在内存，重启需重新连接。该非官方接口可能随账号、固件或服务更新失效；连续读取失败后停止接收并显示原因，不保证与小爱原有语音回答完全无冲突。

如果官方网页登录不能取得会话，可展开高级接入，输入自己 `micoapi` 服务的 `userId=…; serviceToken=…` Cookie。不要使用米家其他服务的令牌。Cookie 不写入音乐库、日志或前端存储；不要向他人提供。官方登录窗口使用临时浏览会话且不能访问本地音乐 IPC。连接后可随时“停止接收”或“断开账号”。

## 从 Bilibili 建立本地曲库

1. 点击 **从 Bilibili 导入**，粘贴视频链接、BV / av 号或 b23.tv 短链接。支持 `zh-Hans` / `zh-Hant` 前缀与 `?p=` 分 P 链接。
2. 点击 **解析视频**，确认标题、作者与分 P，选择一段音轨。
3. 点击 **下载并加入曲库**。可以关闭弹窗，主界面会显示排队、下载字节、入库结果；支持取消和失败重试。

原生 Rust 解析视频信息，选择独立 AAC 音轨，保存为 **M4A**，不会下载视频画面或重新编码，不需要 Python、yt-dlp 或 FFmpeg。单次一段、最多排队 10 项，一次下载一项，单文件上限 1 GB。完成后校验 MP4 容器、原子保存，再把标题与来源写入曲库；未完成文件不会加入曲库，同一分 P 已入库时不会重复下载。任务列表保留本次运行最近 30 项，退出应用中断未完成任务；已完成音乐重启保留。

下载位置显示在弹窗“账号与保存位置”中，默认为应用数据目录的 `downloads/`。这里只支持普通投稿视频中可访问的独立 AAC 音轨，暂不支持直播、番剧、整收藏夹或无独立音轨的视频。公开视频可以匿名解析；需要登录时，可在官方页面登录 Bilibili，并点击“完成登录”。登录只对本次运行有效。访问受限、风控、付费或区域限制会报告错误，不绕过权限。请仅保存你有权下载的内容。M4A 是否能在具体音响固件播放仍需实机确认，本版本不提供 MP3 转码。

接口参考：[XiaoMusic 对话适配实现](https://github.com/hanxi/xiaomusic/blob/main/xiaomusic/conversation.py)、[MiService 的 MiNA 设备接口](https://github.com/Yonsm/MiService/blob/master/miservice/minaservice.py)、[yt-dlp 的 Bilibili 解析器](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/bilibili.py)。这些仅用于核对接口，不是运行依赖。

## 开发运行

需要 Node.js 22+、Rust stable 和对应平台的 Tauri 2 构建环境：

- **Windows 10 / 11**：Microsoft C++ Build Tools（Desktop development with C++、Windows SDK）和 Microsoft Edge WebView2。
- **macOS 11+**：Xcode / Command Line Tools。Apple Silicon 和 Intel 均有打包目标。

详见 [Tauri 2 官方环境配置](https://v2.tauri.app/start/prerequisites/)。

```sh
npm ci
npm run dev
```

界面位于 `frontend/`，无前端构建框架或开发服务器。通过 `core.invoke` 调用 Rust 原生命令；直接用浏览器打开 HTML 不具备控制功能。

若本项目存在 `.tools/cargo` 与 `.tools/rustup`，启动脚本会使用项目内 Rust 工具链；否则使用系统 PATH 中的 Rust。

## 打包

在对应平台执行：

```sh
# 当前系统的默认安装包
npm run build

# Windows：NSIS 安装程序
npm run build -- --bundles nsis

# macOS：应用与 DMG
npm run build -- --bundles app,dmg
```

产物位于 `src-tauri/target/release/bundle/`。macOS 可额外安装 `x86_64-apple-darwin` 或 `aarch64-apple-darwin` Rust 目标，然后传 `--target` 构建对应架构。

若 macOS 的 DMG 装饰阶段被 Finder / 已打开的镜像占用，先退出临时镜像里的应用并推出该镜像，再执行 `CI=true npm run tauri -- bundle --bundles dmg`。这是 Tauri 自带的无界面打包模式，跳过 Finder 窗口装饰。

`.github/workflows/desktop.yml` 提供 Windows x64、macOS Intel、macOS Apple Silicon 的测试和打包矩阵，生成可下载构建产物，不自动发布。上传到 GitHub 仓库后才能实际运行该工作流。

当前配置未包含发行证书或 Apple 公证。面向他人分发前，可按 [Windows 签名](https://v2.tauri.app/distribute/sign/windows/)与 [macOS 签名 / 公证](https://v2.tauri.app/distribute/sign/macos/)配置证书；本地开发不依赖签名证书。

## 测试

```sh
npm run check:ui
npm run test:core
npm test
npm run lint
npm run fmt -- --check
```

核心测试包含真实本地 HTTP / SOAP 模拟音箱、Pro 描述识别、SSDP 响应解析与多目标查询、导入持久化、文件身份检查、媒体 HEAD / Range / 416、播放控制、队列结束判断及服务关闭；另外覆盖语音作用域、时间过滤、重复记录、歌名歧义、目标音响绑定、Bilibili WBI 签名、音轨选择、真实 HTTP 下载队列、取消重试、残缺内容拒绝、凭据域隔离与下载来源持久化。不会向真实音箱发送播放命令。

## 连接排查

**搜索不到音箱**：确认已开启 DLNA、处于同一局域网，避免访客 Wi-Fi、AP 隔离；暂时断开影响本地路由的 VPN。自动发现使用 UDP 组播 `239.255.255.250:1900`。可手动添加 SSDP `LOCATION` 中的完整设备描述 URL，只有 IP 地址不够，端口与路径不能猜测。

**找到设备但不能播放**：允许应用接受局域网入站连接。Windows 防火墙选择私人网络；macOS 允许本地网络访问和入站连接。媒体服务使用随机 TCP 端口，可在界面的诊断信息查看。确认音乐仍在原位置，先用 MP3 排除格式问题。

**进度、音量、自动下一首不可用**：这些操作依赖音箱的 UPnP 服务。只有正常获得播放、进度与结束状态时才自动续播；停止、定位读取失败或检测到其他控制器接管时不会依据旧进度自动切歌。歌曲过短或不报告时长时可手动切歌。

## 数据与访问范围

音乐库索引存放在系统应用数据目录中的 `com.localmusic.xiaomi-dlna/library.json`，macOS 通常在 `~/Library/Application Support/` 下，Windows 通常在 `%APPDATA%` 下。原文件留在导入位置。Bilibili 下载音频由应用保存在同目录下的 `downloads/`，索引中保留来源链接；只记录所选公开视频页，不记录带签名的临时下载地址。

控制入口仅为本地桌面 IPC，没有局域网管理网页或控制 API。HTTP 只提供带随机会话令牌的已导入歌曲，支持单段 Range。应用重启后旧音乐链接失效。设备描述和控制地址仅允许本机 / 局域网地址，不跟随重定向，不使用系统代理。

## 代码结构

```text
frontend/                 中文桌面界面
src-tauri/src/desktop.rs   Tauri 命令、原生选择器、生命周期
src-tauri/src/engine.rs    状态、播放队列、自动发现与轮询
src-tauri/src/dlna.rs      SSDP、设备描述、UPnP SOAP
src-tauri/src/library.rs   本地文件索引与持久化
src-tauri/src/media.rs     带会话令牌的 HTTP 音乐流
src-tauri/src/voice.rs     小爱会话、新指令轮询与本地命令解析
src-tauri/src/bilibili.rs  视频解析、WBI、音轨下载队列与原子入库
src-tauri/src/online.rs    有界在线响应、Cookie 范围与网络超时
src-tauri/tests/           跨模块真实 HTTP 集成测试
```

`xiaomi_dlna/` 与 `tests/` 保留之前的 Python 版本供参考，不参与桌面应用运行或打包；[旧版说明](docs/LEGACY_PYTHON.md)仅适用于旧版。两者音乐库独立，桌面版请重新导入原音乐文件。
