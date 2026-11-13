# 影视直传 M0 开发与验收记录

更新：2026-11-12。代码与后续开发目录：`xiaomi-dlna` 仓库。

2026-11-13 后续进展：M0 代码已收录在 `ed86693`；新增稳定性、恢复与诊断见 [S3 进展](VIDEO_M1_PROGRESS.md)。下方保留 M0 原始验证记录。

当前状态：视频直传的代码和模拟设备闭环已实现；Windows + 创维 A7H 的目标设备验收尚未完成。不能据自动化测试声称 A7H 已支持主力 4K REMUX、HDR、特定音轨或字幕。

## 阶段进度

| 阶段 | 已完成 | 待完成 |
| --- | --- | --- |
| S0 音乐基线 | 变更前音乐 UI、Rust 与 HTTP/SOAP 集成基线；隔离测试资料 | 实际音箱、A7H 固件和家庭网络记录 |
| S1 最小隔离边界 | 默认关闭的影视开关；共用控制锁；owner revision / session generation；旧音乐入口保护 | Windows 原生回归 |
| S2 直传闭环 | 文件选择、受控 FFprobe、候选电视、计划与明确交接、原文件 HTTP、DLNA 控制、独立视频轮询、最小影视 UI | A7H 1080p 与代表性 4K 出画出声；实机音乐→视频→音乐 |
| M0 出口 | 自动化及本机构建结果见下文 | S2 真机门槛通过前，状态保持“待验收” |

后续 S3 高码率整片稳定性、Windows 安装与升级验证，S4 续播/字幕/音轨体验，S5 按失败样例加入兼容处理，不在本次实现范围内。

## 启动

在仓库根目录运行，要求已有项目所需的 Rust、Node 与系统构建工具：

```sh
npm ci
npm run dev:video
```

`dev:video` 校验当前平台 FFprobe 并暂存到 `src-tauri/binaries/`，随后在子进程设置 `XIAOMI_DLNA_VIDEO=1` 启动 Tauri。不需要全局安装 FFmpeg。普通 `npm run dev` / `npm run build` 保持影视默认关闭；缺少 FFprobe 不影响音乐启动。

使用步骤：进入“影视”→选择 MP4、MKV 或 M2TS→等待分析→选择候选电视→“检查并准备”→明确试播。当前正在播放音乐时，需要确认停止当前内容。更换页面、分析文件和选择候选电视都不会打断音乐。

播放时底部视频控制保持可见；暂停和 Seek 依据设备公布的动作能力启用，电视可能仍拒绝特定文件的操作。回到音乐页点击歌曲，选择音乐设备并确认后，才停止影片并开始音乐。

### 构建预览应用

```sh
# Apple Silicon Mac；Intel Mac 改用 x86_64-apple-darwin
npm run build:video -- --target aarch64-apple-darwin --bundles app
```

构建环境变量不会永久写进安装包。直接双击预览应用仍默认隐藏影视；Mac 可显式启动：

```sh
XIAOMI_DLNA_VIDEO=1 "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/小米音乐投送.app/Contents/MacOS/xiaomi-dlna-desktop"
```

使用开发版或预览版时只保留一个应用实例。关闭后以普通方式重新启动，即恢复默认音乐界面。退出会关闭当前文件服务。

Windows 在原生 Windows x64 环境构建：

```powershell
npm ci
npm run build:video -- --target x86_64-pc-windows-msvc --bundles nsis
# 调试直接使用 npm run dev:video；运行安装后的预览版前可设置：
$env:XIAOMI_DLNA_VIDEO = '1'
# 随后从同一 PowerShell 启动安装目录中的应用 EXE。
```

`.github/workflows/video-m0.yml` 提供三平台独立预览产物，名称为 `video-m0-preview-*`，不进入正式 Release 发布步骤。本轮未推送或触发远程构建。普通发行配置未添加 FFprobe。

## 已实现的边界

- 文件仅保留本次会话内的引用；不迁移音乐库、不建立视频持久化索引、不复制影视文件。FFprobe 读取容器、视频/音频/字幕信息；UI 不提供尚未实现的轨道切换按钮。
- FFprobe 使用固定参数、本地文件协议、容器白名单、输出上限、超时和取消回收。工具路径由 Rust 解析，不接收前端任意路径或 shell 参数。三平台来源、精确包版本、原始二进制 SHA-256 固定在 `scripts/video-tools.json`。
- 文件服务复用原有监听器，但视频使用独立的随机会话 URL。支持 GET / HEAD、字节范围、416 和大于 4 GiB 的偏移；实际传输原始文件字节。暂停不会使地址过期。显式 Stop、交接和退出撤销授权并终止对应活动读取。
- 设备宣称的编码/控制能力仅作为证据，不据此保证能解码。计划以“试播原文件”呈现；SOAP 命令已接受、电视已取流和播放状态分别记录，单独一次 HEAD 不算已取流。
- 新旧播放入口共用控制锁。准备计划与候选设备不覆盖音乐选择；显式交接会再次检查所有者、设备、源文件、路由和本地元数据。过期计划/旧会话不能操作后来的播放。
- 视频期间，桌面旧音乐控制、手机、语音和自动下一首均不能隐式接管。明确返回音乐保留原曲库和队列模式。Stop 或部分启动失败后保留“不确定”状态，不自动重放 Play 或假装回滚成功。
- 视频不会触发音乐的自然结束判断。M0 对 `STOPPED` / `NO_MEDIA_PRESENT` 保守处理：可能是跳转、缓冲或片尾，保留会话与文件授权，并等待新状态或用户点击“停止影片”。因此片尾不会自动释放会话，也没有自动连播；这用于避免 Seek 后短暂停止造成断流。

## 本轮验证

以下检查在迁移后的 `xiaomi-dlna` 原仓库执行，基线为 `a486ba7f0ec3f3318e1e865b706676664ae7f42a`（0.9.0）。改动保留在本地工作区，未提交、推送或发行。

| 检查 | 本机结果 |
| --- | --- |
| Rust 默认特性完整套件 | 140 项模块测试 + 8 项 HTTP/SOAP 集成测试通过 |
| FFprobe 原生探测 | 显式执行默认忽略的样片测试，1 项通过 |
| 原生凭据跨进程回归 | 1 项编排测试通过，4 个子进程完成写/读/删/确认空 |
| Node + React | 11 + 72 项通过；包含原有音乐及新增视频交互 |
| 类型、ESLint、格式、三界面构建 | 通过；Rust fmt 与 all-targets Clippy `-D warnings` 通过 |
| 浏览器音乐回归 | 桌面音乐、弹窗焦点/Escape、键盘/窄屏、手机与保留浏览器界面、生产 CSP 均通过 |
| 浏览器视频闭环 | 准备不打断音乐、明确接管、失败后恢复、切回音乐、候选菜单、390px 窄屏通过 |
| 严格 UI 静态审计 | 0 errors / 0 warnings；DESIGN lint 0 errors，16 项既有警告无新增 |
| Apple Silicon 打包 | `.app` 构建成功；资源签名、版本、主程序/FFprobe 架构、工具清单和声明文件验证通过；包内工具实际解析样片通过 |

应用位于 `src-tauri/target/aarch64-apple-darwin/release/bundle/macos/小米音乐投送.app`，未替换或启动用户现有应用。截图位于 `test-results/react/video-*.png`，属于模拟数据的 UI 检查；其中 A7H 名称及 4K 文件信息不代表连接过真实电视。

所有 HTTP/SOAP 接收端均为隔离本机测试服务；浏览器 IPC 为固定测试夹具，不连接用户真实音箱或账号。没有将默认忽略的测试计入普通套件通过数；探测与凭据测试单独显式执行。

回归覆盖：音乐导入、命名、移除、顺序/循环与自动下一首、源文件变更、手机配对/播放/导入、语音设备绑定、登录会话边界、旧设备切换；视频原字节读取、暂停/恢复/停止/Seek、显式跨媒体往返、停止失败、部分启动失败、并发启动、过期状态、路由失败和非法元数据不打断原播放。

```sh
npm run typecheck
npm run check:ui
npm run test:ui
npm run fmt:ui
npm run build:ui
npm run prepare:video
node scripts/cargo.mjs test --manifest-path src-tauri/Cargo.toml --locked
node scripts/cargo.mjs test --manifest-path src-tauri/Cargo.toml --locked --test video_probe_native -- --ignored
node scripts/cargo.mjs test --manifest-path src-tauri/Cargo.toml --locked --lib native_process_round_trip -- --ignored
npm run lint
node scripts/cargo.mjs fmt --manifest-path src-tauri/Cargo.toml -- --check
npm run test:browser
npm run test:browser:video
```

浏览器测试需要 Playwright Chromium；也可通过 `CHROMIUM_PATH` 指定本机 Chrome。测试创建隔离上下文，不使用个人浏览器资料。原生凭据回归只使用随机命名空间的模拟值，并在测试结束清除。

原生 FFprobe 测试使用仓库内自行生成的极小 Matroska/PNG 样片，验证真实子进程及容器解析；该样片不作为 H.264/HEVC、HDR 或电视兼容性证据。

## 已知限制与待验收清单

不包含转码、换封装、ISO/BDMV、字幕烧录、外挂字幕投送、电脑端切轨、续播、后台唤醒管理或手机影视遥控。4K 与无损音轨按原字节发送，但电视内置扬声器的实际解码、下混、输出与 HDR 显示必须实测。

当前 Apple Silicon FFprobe 包实际报告 `n4.4.1`，不是 wrapper 的版本号；不能假设不同平台工具功能完全相同，也不据探测结果认证 Dolby Vision 等格式。正式分发前需完成各平台工具与对应许可/源码材料核查，见 [工具来源](third-party/ffprobe.md)。

| 必验项 | 状态 | 需记录 |
| --- | --- | --- |
| Windows x64 原生构建、安装与 WebView | 待验证 | OS、安装包版本、日志；Mac 交叉检查受缺少 Windows SDK 限制 |
| A7H 1080p H.264/AAC MP4 | 待提供样片和设备 | 固件、接收应用、出画出声、暂停/恢复/停止、Seek |
| A7H 主力 4K MKV/REMUX | 待提供样片和设备 | 大小、码率、HDR、默认音轨/字幕、取流与缓冲表现 |
| 真机音乐→视频→音乐 | 待验证 | 原音箱、电视、显式交接、音乐队列/偏好保持 |
| 长片完整播放与高码率压力 | S3 待验证 | 网络介质、持续吞吐、峰值、内存、结束与退出 |

M0 的目标设备门槛需要前三项相关播放测试及跨媒体往返通过。某个文件失败应保留其容器/编码/网络证据，不自动降低画质，也不把成功发送 SOAP 当成电视播放成功。
