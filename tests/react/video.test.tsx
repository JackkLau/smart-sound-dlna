import React from 'react';
import '@testing-library/jest-dom/vitest';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../../frontend/src/App.jsx';
import type { VideoInfo, VideoSnapshot } from '../../frontend/src/video/types';

const entry: VideoInfo = {
  id: 'film',
  title: '测试影片.mkv',
  format: 'mkv',
  size: '8589934592',
  duration_ms: 120000,
  video_codec: 'hevc',
  width: 3840,
  height: 2160,
  pixel_format: 'yuv420p10le',
  color_transfer: null,
  can_burn_hdr10: false,
  audio_tracks: [{ index: 1, codec: 'aac', language: 'zho', title: null, channels: 2 }],
  subtitle_tracks: [],
};
const initialVideo = (patch: Partial<VideoSnapshot> = {}): VideoSnapshot => ({
  enabled: true,
  owner_kind: 'audio',
  entry: null,
  probe: null,
  plan: null,
  session: null,
  owner_revision: 1,
  music_active: true,
  error: null,
  ...patch,
});
const devices = [
  { id: 'speaker', name: '客厅小爱', address: '192.168.1.5', is_xiaomi: true },
  { id: 'tv', name: '创维 A7H', address: '192.168.1.6', is_xiaomi: false },
];
const music = () => ({
  tracks: [{ id: 'song', title: '原来的歌曲', filename: 'song.mp3', size: 1000, format: 'mp3' }],
  devices,
  selected_device_id: 'speaker',
  device_connection: 'connected',
  current_track_id: 'song',
  transport_state: 'PLAYING',
  position: 12,
  duration: 180,
  volume: 20,
  warnings: [],
  status_error: null,
  queue_mode: 'repeat-all',
  discovering: false,
  discovery_error: null,
  playback_context: {
    active_kind: 'audio',
    owner_revision: 1,
    music_control_blocked_reason: null as string | null,
  },
});
const active = (phase = 'playing') =>
  initialVideo({
    entry,
    music_active: false,
    owner_kind: 'video',
    owner_revision: 2,
    session: {
      id: 'session',
      generation: 2,
      media_id: entry.id,
      device_id: 'tv',
      title: entry.title,
      phase,
      position_ms: 10000,
      duration_ms: 120000,
      can_pause: true,
      can_seek: false,
      command_accepted: true,
      http_requested: false,
      transport_state: 'PLAYING',
      error: null,
    },
  });
const settle = () =>
  act(async () => {
    await Promise.resolve();
  });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (value: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function setup({
  video = initialVideo(),
  enabled = true,
  operation,
}: {
  video?: VideoSnapshot;
  enabled?: boolean;
  operation?: (command: string, args: Record<string, unknown>) => unknown;
} = {}) {
  let data = structuredClone(video);
  let audio = music();
  let nextVideoRead: ReturnType<typeof deferred<VideoSnapshot>> | null = null;
  if (data.session)
    audio = {
      ...audio,
      current_track_id: '',
      transport_state: 'STOPPED',
      playback_context: {
        active_kind: 'video',
        owner_revision: 2,
        music_control_blocked_reason: '当前正在影视播放，请先切换。',
      },
    };
  const invoke = vi.fn(async (command: string, args: Record<string, unknown> = {}) => {
    if (command === 'get_config')
      return { formats: ['mp3'], max_tracks: 20000, video_casting_enabled: enabled };
    if (command === 'get_state') return structuredClone(audio);
    if (command === 'get_features')
      return {
        voice: { connected: false, listening: false, devices: [] },
        downloads: { authenticated: false, jobs: [] },
        xiaomi_login: { phase: 'idle' },
        bilibili_login: { phase: 'idle' },
      };
    if (command === 'get_video_state') {
      if (nextVideoRead) {
        const read = nextVideoRead;
        nextVideoRead = null;
        return read.promise;
      }
      return structuredClone(data);
    }
    const result = await operation?.(command, args);
    if (result !== undefined) return result;
    if (command === 'video_magnet_status') return null;
    if (command === 'forget_recent_video')
      data = {
        ...data,
        recent_videos: data.recent_videos?.filter((item) => item.id !== args.mediaId),
      };
    else if (command === 'select_recent_video')
      data = {
        ...data,
        entry: null,
        plan: null,
        probe: { task_id: 'recent-probe', status: 'running', error: null },
      };
    else if (command === 'pick_video_file') data = { ...data, entry, probe: null, plan: null };
    else if (command === 'prepare_video_play')
      data = {
        ...data,
        plan: {
          id: 'plan',
          media_id: String(args.mediaId),
          device_id: String(args.deviceId),
          subtitle: args.subtitle as NonNullable<VideoSnapshot['plan']>['subtitle'],
          mode: 'trial',
          reason: '设备声明未知，可试播。',
          can_pause: true,
          can_seek: false,
        },
      };
    else if (command === 'start_video_play') data = active('awaiting_evidence');
    else if (command === 'control_video')
      data = {
        ...data,
        session: data.session
          ? { ...data.session, phase: args.action === 'stop' ? 'stopped' : 'paused' }
          : null,
      };
    else if (command === 'start_music_play') {
      audio = {
        ...music(),
        playback_context: {
          active_kind: 'audio',
          owner_revision: 3,
          music_control_blocked_reason: null,
        },
      };
      data = initialVideo({ owner_revision: 3 });
      return structuredClone(audio);
    } else if (command === 'recheck_video_session') data = active();
    else if (command === 'export_video_diagnostics') return true;
    else if (command === 'play_track') return audio;
    else if (command === 'discover_devices') return audio;
    else throw new Error('Unexpected command: ' + command);
    return structuredClone(data);
  });
  render(<App invoke={invoke} />);
  return {
    invoke,
    calls: (command: string) => invoke.mock.calls.filter(([name]) => name === command),
    setVideo: (patch: Partial<VideoSnapshot>) => {
      data = { ...data, ...patch };
    },
    async ready() {
      await settle();
      expect(screen.getByText('客户端已就绪')).toBeVisible();
    },
    async open() {
      fireEvent.click(screen.getByRole('button', { name: '影视' }));
      await settle();
    },
    async tick() {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
    },
    pauseVideoRead() {
      const read = deferred<VideoSnapshot>();
      nextVideoRead = read;
      return read;
    },
  };
}
async function chooseTelevision() {
  fireEvent.keyDown(screen.getByRole('combobox', { name: '候选电视' }), { key: 'ArrowDown' });
  await settle();
  fireEvent.click(screen.getByRole('option', { name: '创维 A7H' }));
  await settle();
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('video casting keeps music boundaries intact', () => {
  it('treats unknown ownership without a video session as an explicit replacement', async () => {
    const app = setup({
      video: initialVideo({ entry, owner_kind: 'unknown', music_active: false }),
      operation: (command) => {
        if (command === 'start_video_play')
          throw new Error('ACTIVE_SESSION_CONFLICT：请先确认原设备停止。');
      },
    });
    await app.ready();
    await app.open();
    await chooseTelevision();
    fireEvent.click(screen.getByRole('button', { name: '检查并准备' }));
    await settle();
    fireEvent.click(screen.getByRole('button', { name: '切换并播放影片' }));
    await settle();
    const dialog = screen.getByRole('dialog', { name: '切换到影片' });
    expect(dialog).toHaveTextContent('将停止当前内容');
    fireEvent.click(within(dialog).getByRole('button', { name: '停止当前内容并播放' }));
    await settle();
    expect(app.calls('start_video_play')[0][1]).toEqual({
      planId: 'plan',
      expectedOwnerRevision: 1,
      replaceCurrent: true,
    });
    expect(within(dialog).getByRole('alert')).toHaveTextContent('请先确认原设备停止。');
    expect(within(dialog).getByRole('alert')).not.toHaveTextContent('ACTIVE_SESSION_CONFLICT');
  });

  it('keeps a probe failure visible across polling while music continues', async () => {
    const app = setup({
      operation: (command) => {
        if (command === 'pick_video_file') throw new Error('FFprobe 未就绪，请重新准备媒体工具。');
      },
    });
    await app.ready();
    await app.open();
    fireEvent.click(screen.getByRole('button', { name: '选择影片' }));
    await settle();
    await app.tick();
    expect(screen.getByRole('alert')).toHaveTextContent('FFprobe 未就绪');
    expect(screen.getByRole('contentinfo', { name: '音乐播放控制' })).toHaveTextContent(
      '原来的歌曲',
    );
    expect(app.calls('control')).toHaveLength(0);
    expect(app.calls('start_video_play')).toHaveLength(0);
  });

  it('cancels only the probe task without sending any playback command', async () => {
    const app = setup({
      video: initialVideo({ probe: { task_id: 'probe-task', status: 'probing', error: null } }),
      operation: (command) =>
        command === 'cancel_video_probe'
          ? initialVideo({ probe: { task_id: 'probe-task', status: 'cancelled', error: null } })
          : undefined,
    });
    await app.ready();
    await app.open();
    expect(screen.getByRole('button', { name: '选择影片' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: '取消读取' }));
    await settle();
    expect(app.calls('cancel_video_probe')[0][1]).toEqual({ taskId: 'probe-task' });
    expect(screen.getByRole('button', { name: '选择影片' })).toBeEnabled();
    expect(app.calls('control')).toHaveLength(0);
    expect(app.calls('start_video_play')).toHaveLength(0);
  });

  it('does not query or expose video when the native feature is off', async () => {
    const app = setup({ enabled: false });
    await app.ready();
    expect(screen.queryByRole('button', { name: '影视' })).toBeNull();
    expect(app.calls('get_video_state')).toHaveLength(0);
    expect(app.calls('video_magnet_status')).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: '播放 原来的歌曲' }));
    await settle();
    expect(app.calls('play_track')).toHaveLength(1);
  });

  it('navigation, picking, candidate selection and preparation never dispatch playback', async () => {
    const app = setup();
    await app.ready();
    await app.open();
    fireEvent.click(screen.getByRole('button', { name: '选择影片' }));
    await settle();
    await chooseTelevision();
    fireEvent.click(screen.getByRole('button', { name: '检查并准备' }));
    await settle();
    expect(app.calls('prepare_video_play')[0][1]).toEqual({ mediaId: 'film', deviceId: 'tv' });
    expect(screen.getByText('原文件直传', { exact: true })).toBeVisible();
    for (const command of ['select_device', 'control', 'start_video_play', 'play_track'])
      expect(app.calls(command)).toHaveLength(0);
    expect(screen.getByRole('contentinfo', { name: '音乐播放控制' })).toHaveTextContent(
      '原来的歌曲',
    );
  });

  it('prepares the chosen PGS track and invalidates the visible plan after changing subtitle mode', async () => {
    const app = setup({
      video: initialVideo({
        entry: {
          ...entry,
          can_burn_hdr10: true,
          subtitle_tracks: [
            ...[8, 9].map((index) => ({
              index,
              codec: 'hdmv_pgs_subtitle',
              language: 'chi',
              title: null,
              channels: null,
            })),
            { index: 4, codec: 'subrip', language: 'eng', title: null, channels: null },
          ],
        },
      }),
    });
    await app.ready();
    await app.open();
    await chooseTelevision();
    const select = async (name: string, option: string) => {
      fireEvent.keyDown(screen.getByRole('combobox', { name }), { key: 'ArrowDown' });
      await settle();
      fireEvent.click(screen.getByRole('option', { name: option }));
      await settle();
    };
    await select('投送字幕', '中文 · PGS · 轨道 9');
    fireEvent.click(screen.getByRole('button', { name: '检查并准备' }));
    await settle();
    expect(app.calls('prepare_video_play')[0][1]).toEqual({
      mediaId: 'film',
      deviceId: 'tv',
      subtitle: { index: 9, mode: 'remux' },
    });
    expect(screen.getByText('字幕版本已准备')).toBeVisible();
    await select('字幕处理方式', '合入画面 · 1080p 兼容模式');
    expect(screen.queryByText('字幕版本已准备')).toBeNull();
    expect(screen.getByText(/HDR 将转为 SDR/)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: '检查并准备' }));
    await settle();
    expect(app.calls('prepare_video_play')[1][1]?.subtitle).toEqual({ index: 9, mode: 'burn' });
    await select('字幕处理方式', '合入画面 · 原分辨率 HDR10');
    expect(screen.queryByText('字幕版本已准备')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '检查并准备' }));
    await settle();
    expect(app.calls('prepare_video_play')[2][1]?.subtitle).toEqual({
      index: 9,
      mode: 'burn_hdr10',
    });
    await select('投送字幕', '英语 · SUBRIP · 轨道 4');
    expect(screen.getByRole('combobox', { name: '字幕处理方式' })).toHaveTextContent('保留原画质');
    expect(screen.queryByText('字幕版本已准备')).toBeNull();
    expect(app.calls('start_video_play')).toHaveLength(0);
    expect(screen.getByRole('contentinfo', { name: '音乐播放控制' })).toHaveTextContent(
      '原来的歌曲',
    );
  });

  it('requires an explicit handoff and does not replay a pending start', async () => {
    const pending = deferred<VideoSnapshot>();
    const app = setup({
      video: initialVideo({ entry }),
      operation: (command) => (command === 'start_video_play' ? pending.promise : undefined),
    });
    await app.ready();
    await app.open();
    await chooseTelevision();
    fireEvent.click(screen.getByRole('button', { name: '检查并准备' }));
    await settle();
    fireEvent.click(screen.getByRole('button', { name: '切换并播放影片' }));
    await settle();
    const dialog = screen.getByRole('dialog', { name: '切换到影片' });
    expect(app.calls('start_video_play')).toHaveLength(0);
    const start = within(dialog).getByRole('button', { name: '停止当前内容并播放' });
    fireEvent.click(start);
    fireEvent.click(start);
    expect(app.calls('start_video_play')).toHaveLength(1);
    expect(app.calls('start_video_play')[0][1]).toEqual({
      planId: 'plan',
      expectedOwnerRevision: 1,
      replaceCurrent: true,
    });
    expect(within(dialog).getByRole('button', { name: '取消' })).toBeDisabled();
    await act(async () => pending.resolve(active('awaiting_evidence')));
    expect(screen.getByRole('contentinfo', { name: '影视播放控制' })).toBeVisible();
    expect(screen.getByText('等待电视播放反馈')).toBeVisible();
    expect(app.calls('start_video_play')).toHaveLength(1);
  });

  it('keeps unknown output honest and disables unsupported seek without touching music', async () => {
    const app = setup({ video: active('remote_state_unknown') });
    await app.ready();
    await app.open();
    expect(screen.getByText(/原文件：尚未收到读取请求/)).toBeVisible();
    expect(screen.getByRole('button', { name: '暂停影片' })).toBeDisabled();
    expect(screen.getByRole('slider', { name: '播放进度' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: '音乐' }));
    await settle();
    expect(screen.getByRole('button', { name: '播放全部' })).toBeDisabled();
    expect(screen.getByRole('button', { name: /已连接 客厅小爱/ })).toBeDisabled();
    expect(screen.getByRole('contentinfo', { name: '影视播放控制' })).toBeVisible();
    expect(app.calls('control')).toHaveLength(0);
  });

  it('returns to music only through the explicit desktop handoff command', async () => {
    const app = setup({ video: active() });
    await app.ready();
    fireEvent.click(screen.getByRole('button', { name: '切换到音乐并播放 原来的歌曲' }));
    await settle();
    expect(app.calls('start_music_play')).toHaveLength(0);
    const dialog = screen.getByRole('dialog', { name: '切换回音乐' });
    fireEvent.click(within(dialog).getByRole('button', { name: '停止当前内容并播放音乐' }));
    await settle();
    expect(app.calls('start_music_play')[0][1]).toEqual({
      trackId: 'song',
      deviceId: 'speaker',
      expectedOwnerRevision: 2,
      replaceCurrent: true,
    });
    expect(app.calls('play_track')).toHaveLength(0);
    expect(screen.queryByRole('dialog', { name: '切换回音乐' })).toBeNull();
  });

  it('keeps a failed start visible inside the handoff dialog and does not claim recovery', async () => {
    const app = setup({
      video: initialVideo({ entry }),
      operation: (command) => {
        if (command === 'start_video_play') throw new Error('电视状态待确认，请先停止。');
      },
    });
    await app.ready();
    await app.open();
    await chooseTelevision();
    fireEvent.click(screen.getByRole('button', { name: '检查并准备' }));
    await settle();
    fireEvent.click(screen.getByRole('button', { name: '切换并播放影片' }));
    await settle();
    const dialog = screen.getByRole('dialog', { name: '切换到影片' });
    fireEvent.click(within(dialog).getByRole('button', { name: '停止当前内容并播放' }));
    await settle();
    expect(within(dialog).getByRole('alert')).toHaveTextContent('电视状态待确认');
    expect(app.calls('start_video_play')).toHaveLength(1);
    expect(app.calls('play_track')).toHaveLength(0);
  });

  it('rejects stale video polls that began before picking a new file', async () => {
    const app = setup();
    await app.ready();
    await app.open();
    const stale = app.pauseVideoRead();
    await app.tick();
    fireEvent.click(screen.getByRole('button', { name: '选择影片' }));
    await settle();
    expect(screen.getByText(entry.title)).toBeVisible();
    await act(async () => stale.resolve(initialVideo()));
    expect(screen.getByText(entry.title)).toBeVisible();
    expect(app.calls('pick_video_file')).toHaveLength(1);
  });
});

describe('video diagnostics and explicit connection checks', () => {
  it('shows bounded transfer evidence without presenting it as playback progress', async () => {
    const state = active();
    Object.assign(state.session!, {
      diagnostics: {
        schema_version: 1,
        elapsed_ms: 180000,
        last_command: { action: 'Play', outcome: 'accepted', elapsed_ms: 45 },
        last_poll_age_ms: 2000,
        consecutive_poll_failures: 0,
        state_source: 'poll',
        transfer: {
          get_requests: 3,
          head_requests: 1,
          range_requests: 2,
          rejected_requests: 0,
          active_streams: 1,
          peak_active_streams: 2,
          bytes_streamed: '8589934592',
          last_request: {
            method: 'GET',
            status: 206,
            range_start: '4294967296',
            range_end: '4295032831',
          },
          revoked: false,
        },
      },
    });
    const app = setup({ video: state });
    await app.ready();
    await app.open();
    fireEvent.click(screen.getByText('连接与传输诊断'));
    expect(screen.getByText('8,589,934,592 字节')).toBeVisible();
    expect(screen.getByText('GET 3 · HEAD 1 · Range 2')).toBeVisible();
    expect(screen.getByText(/4294967296–4295032831/)).toBeVisible();
    expect(screen.getByText(/不代表电视已收完或已播放/)).toBeVisible();
    expect(app.calls('control_video')).toHaveLength(0);
  });

  it('rechecks only on request, blocks duplicates and preserves a failed check for retry', async () => {
    const pending = deferred<VideoSnapshot>();
    let first = true;
    const app = setup({
      video: active('remote_state_unknown'),
      operation: (command) => {
        if (command === 'recheck_video_session' && first) {
          first = false;
          return pending.promise;
        }
      },
    });
    await app.ready();
    await app.open();
    const check = screen.getByRole('button', { name: '重新检查连接' });
    fireEvent.click(check);
    fireEvent.click(check);
    expect(app.calls('recheck_video_session')).toHaveLength(1);
    expect(check).toBeDisabled();
    await act(async () => pending.reject(new Error('REMOTE_STATE_UNKNOWN：电视尚未恢复连接。')));
    await app.tick();
    expect(screen.getByRole('alert')).toHaveTextContent('电视尚未恢复连接。');
    expect(app.calls('recheck_video_session')).toHaveLength(1);
    fireEvent.click(check);
    await settle();
    expect(app.calls('recheck_video_session')[1][1]).toEqual({
      sessionId: 'session',
      expectedGeneration: 2,
    });
    expect(screen.getByRole('button', { name: '暂停影片' })).toBeEnabled();
    expect(app.calls('start_video_play')).toHaveLength(0);
    expect(app.calls('control_video')).toHaveLength(0);
    expect(app.calls('play_track')).toHaveLength(0);
  });

  it('treats native export cancellation honestly and never sends a destination path', async () => {
    const pending = deferred<boolean>();
    let first = true;
    const app = setup({
      video: active(),
      operation: (command) => {
        if (command === 'export_video_diagnostics' && first) {
          first = false;
          return pending.promise;
        }
      },
    });
    await app.ready();
    await app.open();
    const save = screen.getByRole('button', { name: '导出诊断' });
    fireEvent.click(save);
    fireEvent.click(save);
    expect(app.calls('export_video_diagnostics')).toHaveLength(1);
    expect(save).toBeDisabled();
    expect(screen.getByRole('button', { name: '重新检查连接' })).toBeDisabled();
    await act(async () => pending.resolve(false));
    expect(screen.queryByText('诊断已保存。')).toBeNull();
    expect(save).toBeEnabled();
    expect(screen.getByRole('button', { name: '暂停影片' })).toBeEnabled();
    fireEvent.click(save);
    await settle();
    expect(screen.getByText('诊断已保存。')).toBeVisible();
    expect(app.calls('export_video_diagnostics')[1][1]).toEqual({
      sessionId: 'session',
      expectedGeneration: 2,
    });
    expect(app.calls('control_video')).toHaveLength(0);
  });

  it('exports a stopped session but cannot recheck its stale playback ownership', async () => {
    const state = active('stopped');
    state.owner_kind = 'none';
    state.owner_revision = 3;
    const app = setup({
      video: state,
      operation: (command) => {
        if (command === 'export_video_diagnostics') throw new Error('保存失败，请选择其他位置。');
      },
    });
    await app.ready();
    await app.open();
    expect(screen.getByRole('button', { name: '重新检查连接' })).toBeDisabled();
    const save = screen.getByRole('button', { name: '导出诊断' });
    expect(save).toBeEnabled();
    fireEvent.click(save);
    await settle();
    expect(screen.getByRole('alert')).toHaveTextContent('保存失败，请选择其他位置。');
    expect(screen.queryByText('诊断已保存。')).toBeNull();
    expect(app.calls('recheck_video_session')).toHaveLength(0);
  });
});

const recent = (count = 1) =>
  Array.from({ length: count }, (_, index) => ({
    id: `recent-${index}`,
    title: `最近影片 ${index + 1}.mkv`,
    format: 'mkv',
    size: '8589934592',
    duration_ms: 120000,
    video_codec: 'hevc',
    last_selected_ms: 1000,
    progress:
      index === 0 ? { position_ms: 45000, duration_ms: 120000, observed_at_ms: 1000 } : null,
  }));

describe('recent video history', () => {
  it('selects a bounded history item for analysis without playback or changing music', async () => {
    const app = setup({ video: initialVideo({ recent_videos: recent(7) }) });
    await app.ready();
    await app.open();
    const list = screen.getByRole('region', { name: '最近影片' });
    expect(within(list).getAllByRole('listitem')).toHaveLength(5);
    expect(list).toHaveTextContent('上次确认位置 0:45 / 2:00');
    expect(within(list).queryByRole('button', { name: /续播/ })).toBeNull();
    fireEvent.click(within(list).getByRole('button', { name: '显示更多影片' }));
    expect(within(list).getAllByRole('listitem')).toHaveLength(7);
    fireEvent.click(within(list).getByRole('button', { name: '重新选择 最近影片 1.mkv' }));
    await settle();
    expect(app.calls('select_recent_video')[0][1]).toEqual({ mediaId: 'recent-0' });
    expect(screen.getByText('正在读取影片信息，不会复制或转换原文件。')).toBeVisible();
    expect(screen.getByRole('contentinfo', { name: '音乐播放控制' })).toHaveTextContent(
      '原来的歌曲',
    );
    expect(app.calls('start_video_play')).toHaveLength(0);
    expect(app.calls('control')).toHaveLength(0);
    expect(app.calls('select_device')).toHaveLength(0);
  });

  it('confirms forgetting history without stopping the active film and restores focus', async () => {
    const app = setup({ video: { ...active(), recent_videos: recent() } });
    await app.ready();
    await app.open();
    fireEvent.click(screen.getByRole('button', { name: '移除记录 最近影片 1.mkv' }));
    let dialog = screen.getByRole('dialog', { name: '移除影片记录' });
    expect(dialog).toHaveTextContent('不会删除原文件或停止当前播放');
    fireEvent.click(within(dialog).getByRole('button', { name: '取消' }));
    expect(app.calls('forget_recent_video')).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: '移除记录 最近影片 1.mkv' }));
    dialog = screen.getByRole('dialog', { name: '移除影片记录' });
    fireEvent.click(within(dialog).getByRole('button', { name: '移除记录' }));
    await settle();
    expect(app.calls('forget_recent_video')[0][1]).toEqual({ mediaId: 'recent-0' });
    expect(screen.queryByRole('dialog', { name: '移除影片记录' })).toBeNull();
    expect(screen.getByRole('heading', { name: '最近影片' })).toHaveFocus();
    expect(screen.getByText('影片记录已移除。')).toBeVisible();
    expect(screen.getByRole('button', { name: '暂停影片' })).toBeEnabled();
    expect(app.calls('control_video')).toHaveLength(0);
  });

  it('keeps a failed removal open and prevents duplicate mutations while saving', async () => {
    const request = deferred<VideoSnapshot>();
    const app = setup({
      video: initialVideo({ recent_videos: recent() }),
      operation: (command) => (command === 'forget_recent_video' ? request.promise : undefined),
    });
    await app.ready();
    await app.open();
    fireEvent.click(screen.getByRole('button', { name: '移除记录 最近影片 1.mkv' }));
    const dialog = screen.getByRole('dialog', { name: '移除影片记录' });
    const remove = within(dialog).getByRole('button', { name: '移除记录' });
    fireEvent.click(remove);
    fireEvent.click(remove);
    expect(remove).toBeDisabled();
    expect(within(dialog).getByRole('button', { name: '取消' })).toBeDisabled();
    expect(app.calls('forget_recent_video')).toHaveLength(1);
    await act(async () => request.reject(new Error('索引已被其他进程修改，请重启后再试。')));
    expect(within(dialog).getByRole('alert')).toHaveTextContent('索引已被其他进程修改');
    expect(remove).toBeEnabled();
    expect(screen.getByRole('region', { name: '最近影片' })).toHaveTextContent('最近影片 1.mkv');
  });

  it('keeps native selection available when history cannot be loaded', async () => {
    const app = setup({
      video: initialVideo({
        recent_videos: [],
        catalog_warning: '影片索引版本不受支持，已暂停记录保存。',
      }),
    });
    await app.ready();
    await app.open();
    expect(screen.getByRole('region', { name: '最近影片' })).toHaveTextContent(
      '影片索引版本不受支持',
    );
    expect(screen.getByRole('button', { name: '选择影片' })).toBeEnabled();
    fireEvent.click(screen.getByRole('button', { name: '选择影片' }));
    await settle();
    expect(app.calls('pick_video_file')).toHaveLength(1);
    expect(screen.getByRole('contentinfo', { name: '音乐播放控制' })).toHaveTextContent(
      '原来的歌曲',
    );
  });
});

describe('影视磁力下载', () => {
  const magnet = 'magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=demo';
  const download = (status: string) => ({
    id: 'job-7',
    status,
    name: '演示资源',
    progressBytes: 1024,
    totalBytes: 2048,
    speedBytesPerSec: 512,
    error: null,
    files:
      status === 'complete'
        ? [
            { index: 4, name: '正片.mkv', size: 2048 },
            { index: 5, name: '花絮.mp4', size: 1024 },
          ]
        : [],
  });
  it('retains the link on picker dismissal and opens only the selected completed movie without switching playback', async () => {
    let attempts = 0;
    let current: ReturnType<typeof download> | null = null;
    const app = setup({
      operation: (command) => {
        if (command === 'video_magnet_status') return current;
        if (command === 'video_magnet_start') {
          if (attempts++ === 0) return null;
          current = download('fetchingMetadata');
          return current;
        }
        if (command === 'video_magnet_open') return initialVideo({ entry });
      },
    });
    await app.ready();
    expect(app.calls('video_magnet_status')).toHaveLength(0);
    await app.open();
    const field = screen.getByRole('textbox', { name: '影片磁力链接' });
    const start = screen.getByRole('button', { name: '选择目录并下载' });
    fireEvent.change(field, { target: { value: 'bad link' } });
    fireEvent.click(start);
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(app.calls('video_magnet_start')).toHaveLength(0);
    fireEvent.change(field, { target: { value: '  ' + magnet + '  ' } });
    fireEvent.click(start);
    await settle();
    expect(field).toHaveValue('  ' + magnet + '  ');
    fireEvent.click(start);
    await settle();
    expect(app.calls('video_magnet_start')[1][1]).toEqual({ magnet });
    expect(screen.getByText('正在获取影片列表')).toBeVisible();
    current = download('downloading');
    await app.tick();
    expect(screen.getByRole('progressbar', { name: '影片下载进度' })).toHaveAttribute(
      'value',
      '1024',
    );
    expect(screen.getByText(/512 B\/s/)).toBeVisible();
    current = download('complete');
    await app.tick();
    fireEvent.click(screen.getByRole('button', { name: '载入影片 正片.mkv' }));
    await settle();
    expect(app.calls('video_magnet_open')[0][1]).toEqual({ id: 'job-7', index: 4 });
    expect(screen.getByRole('heading', { name: '影片文件' })).toHaveFocus();
    expect(screen.getByRole('contentinfo', { name: '音乐播放控制' })).toHaveTextContent(
      '原来的歌曲',
    );
    expect(app.calls('start_video_play')).toHaveLength(0);
    expect(app.calls('control')).toHaveLength(0);
  });

  it('cancels the active task by ID and ignores an older in-flight progress read', async () => {
    const oldRead = deferred<ReturnType<typeof download>>();
    let reads = 0;
    const app = setup({
      operation: (command) => {
        if (command === 'video_magnet_status')
          return reads++ === 0 ? download('downloading') : oldRead.promise;
        if (command === 'video_magnet_cancel') return download('cancelled');
      },
    });
    await app.ready();
    await app.open();
    await app.tick();
    fireEvent.click(screen.getByRole('button', { name: '取消下载' }));
    await settle();
    expect(app.calls('video_magnet_cancel')[0][1]).toEqual({ id: 'job-7' });
    await act(async () => {
      oldRead.resolve(download('downloading'));
    });
    expect(screen.getByText('下载已取消')).toBeVisible();
    expect(screen.queryByRole('progressbar', { name: '影片下载进度' })).toBeNull();
  });
});
