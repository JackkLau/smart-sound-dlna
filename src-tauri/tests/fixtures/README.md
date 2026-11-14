# Generated video fixture

generated-video.mkv is a 16 by 16 solid-colour, two-frame Matroska fixture generated for this project. It contains PNG frames, a V_MS/VFW/FOURCC track with MPNG codec tag, and a two-second duration. It contains no third-party footage, audio, or personal metadata.

The native FFprobe test verifies a real external process and real container parsing. This tiny fixture does not demonstrate TV compatibility, H.264/HEVC support, HDR rendering, or high-bitrate performance.

`generated-subtitles.mkv` 为自行生成的 1.25 秒黑色视频和正弦波，包含一条英文 SubRip 测试文本以及一条标记为中文的 PGS 白矩形。字幕在 1 秒时均已消失。测试验证从模拟电视收到的 MKV 中解码出的像素、轨道和原始音视频包，无商业影片内容。

`generated-subtitles-hdr10.mkv` 使用上述黑场夹具生成 HEVC Main 10、BT.2020/PQ 版本，保留同一音轨和字幕，附固定的母版显示与亮度元数据，用于验证 HDR10 烧录。
