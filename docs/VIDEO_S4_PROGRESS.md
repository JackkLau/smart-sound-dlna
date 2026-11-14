# S4.1 最近影片与可靠进度

更新：2026-11-13。实现与验证均在当前 `xiaomi-dlna` 仓库完成，保留此前 M0 / S3 的工作；未提交、推送或发布。

本轮完成 S4.1 的本机软件闭环：独立索引、近期影片、可靠位置保存、重启后的历史选择。M0 的 Windows + A7H 真机验收与 S3 整片测试仍未完成，不据此宣称 M0 / M1 已通过目标设备验收。

## 用户可以做什么

运行 `npm run dev:video` 后，在影视页面选择并分析本地影片。“最近影片”会保存最近 100 部，初始显示 5 部，通过“显示更多影片”逐批展开。

- 重新选择：核对原文件身份、大小和修改时间，并再次运行包内 FFprobe；不会自动准备、播放、跳转或切换音乐设备。
- 查看位置：显示电视最近可靠回报并已保存的位置。HTTP 字节数、接受的 Play/Seek 指令和本地计时均不作为位置证据。
- 移除记录：使用现有确认弹窗，移除该片历史及进度；保留原文件，也不会停止活动影片或撤销其文件授权。
- 重启应用：恢复近期列表与位置，不恢复活动会话、文件授权、电视选择或播放指令。

目前没有“继续播放”按钮。已有的手动播放与按设备声明开放的 Seek 行为保持原样；S4.2 的自动定位续播必须等具体文件与设备组合完成实际 Seek 验证。

## 持久化边界

文件为应用数据目录下独立的 `video-library.json`，schema version 为 `1`。音乐 `library.json` 的格式、加载和写入规则不迁移。影视开关关闭时不读取或创建影片索引。

原生记录包含不透明影片 ID、规范化本地路径、平台文件身份、大小、修改时间、受限影片摘要、最近选择时间，以及可靠进度和对应设备标识。路径及设备绑定仅留在原生索引中；前端快照只接收摘要与位置，不接收路径或设备标识。它是本机索引，不上传、不读取真实账号、不共享给手机音乐接口。

索引最多 100 条、4 MiB。同一文件身份未变时重复选择复用影片 ID 和位置；文件替换、修改或移动后，历史选择会拒绝并要求重新用原生选择框选片。显式重新导入变更文件会建立新 ID、清除该路径的旧进度。旧记录不会被用来直接生成取流授权。身份检查会在重选前和 FFprobe 完成后进行。

写入采用同目录临时文件、文件同步、原子替换；写入失败保留已提交的内存与磁盘状态，并在影片区域说明未保存。读取遇到损坏、未知版本、过大、链接或不可读索引时，不覆盖原文件；停止历史保存，仍允许原生选片直传及音乐操作。可备份并修复索引后重启。

每次写入也比较已加载的文件内容；检测到外部修改时拒绝覆盖并提示重启。这不是跨进程事务锁：同一数据目录仍不支持同时运行多个写入实例。原子替换也不构成断电持久性保证。测试不操作用户真实数据目录。

## 什么位置可以保存

必须同时满足：

1. 仍是本次活动影片的有效会话代际，授权尚未撤销。
2. 电视返回的 URI 精确等于本次授权 URL。
3. 电视状态为 `PLAYING` 或 `PAUSED_PLAYBACK`。
4. 时间可解析，时长大于零且不超过 99:59:59，位置不超过时长。

首次可靠读数可以保存。播放中的变化以至少 30 秒间隔写检查点；普通轮询不反复写盘。明确暂停、停止和音乐/影片交接时，保存最近已确认的读数。重复的相同位置不重复写入。保存错误不会使播放控制失败。

接受 Seek 后显示的目标不是确认位置，必须等电视可靠回报后才可保存。缺失 URI、无效时间、短暂 `STOPPED`、缓冲、断连或其他来源接管都不会制造新的可靠位置。自然结束仍不自动连播。

本轮没有改变关窗、退出和后台行为，也没有新增退出时远端查询。正常轮询及保存时，异常退出可能丢失约 30 秒的最新记录；失联或保存失败时可能保留更早的位置。移除记录后，迟到的进度保存不会重新创建它。

## 代码与接口

| 位置 | 责任 |
| --- | --- |
| `src-tauri/src/video_catalog.rs` | 版本校验、容量限制、原子发布、近期排序与可靠位置存储 |
| `src-tauri/src/video.rs` | 可持久化的原生文件身份快照与重新核验 |
| `src-tauri/src/engine_video.rs` | 重选分析、会话证据筛选、写入节流、暂停/停止/交接检查点 |
| `src-tauri/src/engine.rs` | 仅在开启影视时加载独立索引 |
| `src-tauri/src/desktop.rs` | 原生命令 `select_recent_video` / `forget_recent_video`，只接受影片 ID |
| `frontend/src/video/RecentVideos.tsx` | 有界列表、可靠位置、确认移除、错误恢复与焦点管理 |
| `frontend/src/video/types.ts` | `recent_videos` / `catalog_warning` 快照扩展；兼容旧测试快照 |

界面沿用 `Dialog`、按钮、样式变量与 `useDesktop.mutate`；不增加第二套动作队列。索引错误不会覆盖播放错误。选片、重选和移除记录不发送播放 SOAP。已有音乐队列在同次运行的影片往返中保持；重启后的音乐模式仍使用原有行为，本轮没有新增音乐设置持久化。

## 2026-11-13 验证证据

| 检查 | 本轮结果 |
| --- | --- |
| 全量 Rust | 167 项模块 + 8 项本地 HTTP/SOAP 集成通过；有依赖原生工具/凭据的测试另行显式执行 |
| 影片引擎 | 31 项非忽略测试通过；新增缺失 URI/无效时间/越界位置/短暂停止、写入节流、明确交接、损坏索引降级、活动影片移除及音乐隔离 |
| 索引 | 7 项通过；含稳定 ID/重启、变更文件清除进度、损坏/未知版本保护、外部修改/写入失败、迟到进度、100 条/4 MiB 上限、非法进度拒绝 |
| 真实 FFprobe | 小型 Matroska 样片解析通过；另通过 Engine 导入 → 重启 → 历史重选 → 再次分析，保持 ID 且没有自动播放 |
| 原生凭据 | 随机独立测试命名空间的跨进程写/读/删/确认空通过；不访问真实账号 |
| Node / React | 16 + 80 项通过；影视组件 18 项，其中新增最近影片的 4 项先失败后通过 |
| 浏览器 | 旧音乐/手机/保留界面回归及影视流程通过；新增列表展开、重选失败/重试/分析、确认取消、移除失败/重试、焦点恢复、390px 窄屏与活动影片继续播放 |
| 类型/风格 | TypeScript、ESLint、Prettier、Rust fmt、Clippy `-D warnings`、三界面生产构建通过 |
| UI 审核 | strict audit 0 errors / 0 warnings；DESIGN lint 0 errors，16 项既有 token 警告无新增 |
| Mac 应用包 | Apple Silicon `.app` 构建通过；资源签名、真实架构、包内 FFprobe 固定来源与无 PATH 样片解析通过；未启动/替换用户应用 |

原生恢复测试已加入 `video-m0.yml` 的各平台预览任务，但本轮没有触发远程 CI。Windows / Intel Mac 的实际执行仍待完成。

可复现命令（仓库根目录）：

```sh
npm run typecheck
npm run check:ui
npm run test:ui
npm run fmt:ui
npm run build:ui
node scripts/cargo.mjs test --manifest-path src-tauri/Cargo.toml --locked
node scripts/cargo.mjs test --manifest-path src-tauri/Cargo.toml --locked --lib packaged_ffprobe_restores_recent_video -- --ignored
node scripts/cargo.mjs test --manifest-path src-tauri/Cargo.toml --locked --test video_probe_native -- --ignored
node scripts/cargo.mjs test --manifest-path src-tauri/Cargo.toml --locked --lib native_process_round_trip -- --ignored
node scripts/cargo.mjs clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
node scripts/cargo.mjs fmt --manifest-path src-tauri/Cargo.toml -- --check
npm run test:browser
npm run test:browser:video
npm run build:video -- --bundles app --target aarch64-apple-darwin
node scripts/verify-video-bundle.mjs 'src-tauri/target/aarch64-apple-darwin/release/bundle/macos/小米音乐投送.app' aarch64-apple-darwin
```

浏览器命令需本机 Playwright 浏览器，或设置 `CHROMIUM_PATH`；本轮使用独立 Chrome 进程与随机端口。截图为 `test-results/react/video-recent.png`、`video-recent-narrow.png`、`video-recent-removal-error.png`。影片标题、A7H 名称和进度均为测试夹具，不是电视实测证据。

## 后续任务

- 保留 [M0 真机清单](VIDEO_M0.md#已知限制与待验收清单) 和 [S3 整片/网络/安装边界](VIDEO_M1_PROGRESS.md)；先验证 Windows + A7H 基线和主力 4K 文件。
- S4.2：结合明确的文件/设备 Seek 实测档案，提供用户主动触发的续播；不从能力声明推断已验证。
- S4.3 已支持在影视页「投送字幕」指定单条内封字幕。保留原画质模式复制视频和全部音轨，将所选字幕写为唯一的默认、强制轨道。电视能否解码 PGS 仍取决于接收器。「合入画面」生成带固定字幕的 1080p H.264 版本，将 PQ/HLG 转为 SDR 并复制音轨。在 Apple Silicon 与 Windows x64 的影视包中，10 位 BT.2020/PQ 片源的 PGS 字幕还可选「合入画面 · 原分辨率 HDR10」，生成原尺寸 HEVC Main 10 影片并复制原音轨。输出使用 HDR10，整片需重新编码。Intel Mac 包内 FFmpeg 版本较旧，仅提供前两种模式。上述模式均在用户点击「检查并准备」后写临时 MKV，可取消，准备完成后手动播放。原文件模式仍可直接播放。最近影片只保存原片。
- 字幕临时文件位于应用数据目录的 `subtitle-work`。新一轮准备会替换旧计划，正在投送的版本由会话持有，停止和取消后释放。异常退出遗留的工作目录在下一次准备字幕版本时回收。建议为保留原画质模式预留接近原片大小的磁盘空间。
- S4.4：播放期间保持唤醒、托盘和退出清理尚未实现，不能默认为电脑已防休眠。
- S4.5：设备固件与兼容性实测档案尚未实现；进度中的本地设备绑定不替代能力或测试档案。
