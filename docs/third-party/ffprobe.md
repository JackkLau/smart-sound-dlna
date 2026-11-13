# FFprobe 工具来源

影视试用版本仅分发独立 FFprobe，不分发 FFmpeg 编码工具，也不使用 Node 作为运行时。

安装来源固定为 @ffprobe-installer/ffprobe 2.1.2。平台包版本与原始二进制 SHA-256 记录在 scripts/video-tools.json，npm 包完整性记录在 package-lock.json。准备脚本先检查包名、版本和二进制校验值，再运行已校验的本机工具。

来源仓库：https://github.com/SavageCore/node-ffprobe-installer

上游 FFmpeg：https://ffmpeg.org/

许可说明：https://ffmpeg.org/legal.html

每个构建产物携带 ffprobe-manifest.json，记录该平台实际工具的 -version、构建配置和 -L 输出。平台包的版本号不是 FFmpeg 项目版本号，不应混用。平台构建的许可和功能选项可能不同，不能仅凭 npm wrapper 的许可字段推断实际二进制许可。

正式对外发行前，依据所选二进制实际输出和上游对应构建，准备适用的许可文本与对应源代码获取方式。当前工具清单用于 M0 内部试播与验证；更新工具必须同时更新精确版本、锁文件和校验值，并重新验证三平台探测。


## 包内自检

`node scripts/verify-video-bundle.mjs <应用包或安装目录> <target>` 验证实际随应用分发的工具，不能用系统 PATH 中的另一个 FFprobe 替代。它读取主程序和工具的 Mach-O/PE 架构、核对固定来源清单与声明文件、以清空 PATH 且过滤环境变量的子进程核对 `-version` / `-L`，并解析仓库中自行生成的 Matroska 样片。执行有超时与输出上限，不启动桌面主程序。

Mac 打包会重签名 FFprobe，因此直接比较成品完整 SHA-256 无法等同于原始工具校验值。自检先核对暂存原始工具的固定 SHA-256，再在临时副本中移除签名；仅归一化重签名遗留的 `__LINKEDIT` 虚拟分配大小，逐字节比较其余代码/数据/元数据，并验证成品应用整体资源签名。不会修改原始工具或应用包。Windows 当前未配置发行签名，自检要求包内工具完整 SHA-256 与固定值一致；未来添加 Authenticode 时应同步设计来源校验，而不是直接跳过检查。

预览工作流在 GitHub 托管的临时 Windows runner 中运行 NSIS 安装到含空格/中文的随机目录，拒绝已有同名安装或正在运行的应用；验证实际安装路径后卸载并清理该临时目录。安装过程不传自动启动标记，也不运行主程序。该步骤不允许在本地电脑或自托管 runner 自动运行。NSIS 的参数规则见[官方命令行文档](https://nsis.sourceforge.io/Docs/Chapter3.html)；当前 Tauri 的静默安装、禁用快捷方式及运行开关见 [CLI 2.12.4 安装器模板](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi)。

新增 CI 步骤尚需实际 Windows 云端执行，脚本及模拟文件布局测试不能代替 Windows 安装、WebView 或电视播放验收。
