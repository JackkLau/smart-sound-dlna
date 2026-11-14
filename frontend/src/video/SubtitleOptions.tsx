import { Select } from '../components.jsx';
import type { VideoInfo, SubtitleSelection, VideoTrackInfo } from './types';

const supported = new Set([
  'hdmv_pgs_subtitle',
  'dvd_subtitle',
  'dvb_subtitle',
  'subrip',
  'ass',
  'ssa',
  'mov_text',
  'webvtt',
  'text',
]);
const languages: Record<string, string> = {
  chi: '中文',
  zho: '中文',
  zh: '中文',
  eng: '英语',
  jpn: '日语',
  kor: '韩语',
};
function trackName(track: VideoTrackInfo) {
  const codec = track.codec === 'hdmv_pgs_subtitle' ? 'PGS' : track.codec?.toUpperCase();
  return [
    track.language && (languages[track.language] || track.language),
    track.title,
    codec,
    '轨道 ' + track.index,
  ]
    .filter(Boolean)
    .join(' · ');
}

export default function SubtitleOptions({
  entry,
  choice,
  disabled,
  onChange,
}: {
  entry: VideoInfo;
  choice: SubtitleSelection | null;
  disabled: boolean;
  onChange: (choice: SubtitleSelection | null) => void;
}) {
  if (!entry.subtitle_tracks.length) return null;
  const supportsHdrBurn = (index: number) =>
    entry.can_burn_hdr10 &&
    entry.subtitle_tracks.some((t) => t.index === index && t.codec === 'hdmv_pgs_subtitle');
  return (
    <>
      <label className="field-label" htmlFor="video-subtitle">
        投送字幕
      </label>
      <Select
        id="video-subtitle"
        value={choice ? String(choice.index) : 'original'}
        disabled={disabled}
        onChange={({ target }: { target: { value: string } }) => {
          if (target.value === 'original') return onChange(null);
          const index = Number(target.value);
          const mode = choice?.mode ?? 'remux';
          onChange({
            index,
            mode: mode === 'burn_hdr10' && !supportsHdrBurn(index) ? 'remux' : mode,
          });
        }}
      >
        <option value="original">原文件 · 由电视选择</option>
        {entry.subtitle_tracks.map((t) => (
          <option key={t.index} value={String(t.index)} disabled={!supported.has(t.codec || '')}>
            {trackName(t)}
          </option>
        ))}
      </Select>
      {choice && (
        <>
          <label className="field-label" htmlFor="video-subtitle-mode">
            字幕处理方式
          </label>
          <Select
            id="video-subtitle-mode"
            value={choice.mode}
            disabled={disabled}
            onChange={({ target }: { target: { value: string } }) =>
              onChange({ ...choice, mode: target.value as SubtitleSelection['mode'] })
            }
          >
            <option value="remux">保留原画质 · 指定默认字幕</option>
            {supportsHdrBurn(choice.index) && (
              <option value="burn_hdr10">合入画面 · 原分辨率 HDR10</option>
            )}
            <option value="burn">合入画面 · 1080p 兼容模式</option>
          </Select>
          <p className="field-help">
            {choice.mode === 'remux'
              ? '复制原视频和音轨，生成临时 MKV。请预留接近原片大小的磁盘空间。若电视仍不显示，可改选合入画面。'
              : choice.mode === 'burn_hdr10'
                ? '按原分辨率重新编码为 10 位 HEVC，固定显示所选字幕，复制全部音轨。整片编码可能需要数小时和较大临时空间。输出为 HDR10，不保留动态 HDR 信息。'
                : '生成带固定字幕的 1080p H.264 影片，HDR 将转为 SDR，保留原音轨。整片编码需要时间和临时磁盘空间。'}
          </p>
        </>
      )}
    </>
  );
}
