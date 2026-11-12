# FFprobe 工具来源

影视试用版本仅分发独立 FFprobe，不分发 FFmpeg 编码工具，也不使用 Node 作为运行时。

安装来源固定为 @ffprobe-installer/ffprobe 2.1.2。平台包版本与原始二进制 SHA-256 记录在 scripts/video-tools.json，npm 包完整性记录在 package-lock.json。准备脚本先检查包名、版本和二进制校验值，再运行已校验的本机工具。

来源仓库：https://github.com/SavageCore/node-ffprobe-installer

上游 FFmpeg：https://ffmpeg.org/

许可说明：https://ffmpeg.org/legal.html

每个构建产物携带 ffprobe-manifest.json，记录该平台实际工具的 -version、构建配置和 -L 输出。平台包的版本号不是 FFmpeg 项目版本号，不应混用。平台构建的许可和功能选项可能不同，不能仅凭 npm wrapper 的许可字段推断实际二进制许可。

正式对外发行前，依据所选二进制实际输出和上游对应构建，准备适用的许可文本与对应源代码获取方式。当前工具清单用于 M0 内部试播与验证；更新工具必须同时更新精确版本、锁文件和校验值，并重新验证三平台探测。
