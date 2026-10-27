import React from 'react';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../../frontend/src/App.jsx';

const track = (id, title = `歌曲 ${id}`) => ({
  id,
  title,
  filename: `${id}.mp3`,
  format: 'mp3',
  size: 2048,
});
const first = track('first', '第一首');
const second = track('second', '第二首');
const speaker = {
  id: 'speaker',
  name: '客厅小爱',
  address: '192.168.1.5',
  model: 'Pro',
  is_xiaomi: true,
  is_xiaomi_pro: true,
};
const snapshot = (patch = {}) => ({
  tracks: [first, second],
  devices: [speaker],
  selected_device_id: 'speaker',
  device_connection: 'connected',
  current_track_id: null,
  transport_state: 'STOPPED',
  position: 0,
  duration: 180,
  volume: 30,
  warnings: [],
  status_error: null,
  queue_mode: 'sequential',
  discovering: false,
  discovery_error: null,
  ...patch,
});
const features = () => ({
  voice: { connected: false, listening: false, devices: [] },
  downloads: { authenticated: false, directory: '/fixture/downloads', jobs: [] },
  xiaomi_login: { phase: 'idle' },
  bilibili_login: { phase: 'idle' },
  selected_device_id: 'speaker',
  selected_device_name: '客厅小爱',
});
const settle = async () =>
  act(async () => {
    await Promise.resolve();
  });
const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};

function setup({ initial = snapshot(), operation } = {}) {
  let data = structuredClone(initial);
  let nextRead = null;
  const set = (patch) => {
    data = { ...data, ...patch };
  };
  const invoke = vi.fn(async (command, args) => {
    if (command === 'get_config')
      return { formats: ['mp3', 'm4a'], max_tracks: 20000, platform: 'linux' };
    if (command === 'get_features') return features();
    if (command === 'get_mobile_receiver')
      return { enabled: false, paired: false, pairing_available: false, entries: [] };
    if (command === 'get_state') {
      const captured = structuredClone(data);
      if (nextRead) {
        const pending = nextRead;
        nextRead = null;
        return pending.promise;
      }
      return captured;
    }
    const result = await operation?.(command, args, { set, get: () => data });
    if (result !== undefined) return result;
    if (command === 'select_device')
      set({ selected_device_id: args.deviceId, device_connection: 'checking' });
    else if (command === 'discover_devices') set({ discovering: false });
    else if (command === 'play_track')
      set({ current_track_id: args.trackId, transport_state: 'PLAYING' });
    else if (command === 'rename_track')
      set({
        tracks: data.tracks.map((item) =>
          item.id === args.trackId ? { ...item, title: args.title } : item,
        ),
      });
    else if (command === 'remove_track')
      set({
        tracks: data.tracks.filter((item) => item.id !== args.trackId),
        ...(data.current_track_id === args.trackId
          ? { current_track_id: null, transport_state: 'STOPPED' }
          : {}),
      });
    else if (command === 'recognize_track_name') return { title: '晴天', source: 'audio_tag' };
    else if (command === 'pick_music_files' || command === 'pick_music_folder') return null;
    else if (command === 'add_device')
      set({ devices: [...data.devices, { id: 'manual', name: '手动音响', address: args.url }] });
    else if (command === 'control') {
      if (args.action === 'play-all')
        set({
          current_track_id: data.tracks[0]?.id,
          queue_mode: 'sequential',
          transport_state: 'PLAYING',
        });
      else if (args.action === 'mode') set({ queue_mode: args.value });
      else if (args.action === 'pause') set({ transport_state: 'PAUSED_PLAYBACK' });
      else if (args.action === 'resume') set({ transport_state: 'PLAYING' });
    } else throw new Error(`Unexpected command: ${command}`);
    return structuredClone(data);
  });
  const view = render(<App invoke={invoke} />);
  return {
    ...view,
    invoke,
    set,
    get: () => data,
    calls: (command) => invoke.mock.calls.filter(([name]) => name === command),
    async ready() {
      await settle();
      expect(screen.getByText('客户端已就绪')).toBeVisible();
    },
    async tick(patch) {
      if (patch) set(patch);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
    },
    pauseNextRead() {
      const pending = deferred();
      nextRead = pending;
      return pending;
    },
  };
}
function click(name) {
  fireEvent.click(screen.getByRole('button', { name }));
}
function edit(title) {
  click(`识别或修改曲名：${title}`);
}
function changeName(value) {
  fireEvent.change(screen.getByLabelText('曲名'), { target: { value } });
}
const dialog = () => screen.getByRole('dialog', { name: '识别或修改曲名' });

beforeEach(() => {
  vi.useFakeTimers();
  sessionStorage.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('React desktop connection and library controls', () => {
  it('enables playback only after confirmation and retries a disconnected selection without auto-playing', async () => {
    const ui = setup({ initial: snapshot({ device_connection: 'checking' }) });
    await ui.ready();
    expect(screen.getByRole('button', { name: '播放 第一首' })).toBeDisabled();
    expect(document.getElementById('device-hint').textContent).toMatch(/确认音响连接/);
    await ui.tick({ device_connection: 'connected' });
    expect(screen.getByRole('button', { name: '播放 第一首' })).toBeEnabled();
    expect(screen.getByRole('button', { name: '播放', exact: true })).toBeEnabled();
    expect(screen.getByRole('button', { name: '搜索其他音响' })).toBeEnabled();
    click(/已连接 客厅小爱/);
    await settle();
    expect(ui.calls('select_device')).toHaveLength(0);
    await ui.tick({ device_connection: 'disconnected', transport_state: 'UNAVAILABLE' });
    expect(screen.getByRole('button', { name: '播放 第一首' })).toBeDisabled();
    expect(document.getElementById('now-subtitle').textContent).toMatch(/已断开/);
    click(/点击重试 客厅小爱/);
    await settle();
    expect(ui.calls('select_device')).toEqual([['select_device', { deviceId: 'speaker' }]]);
    expect(document.getElementById('device-hint').textContent).toMatch(/确认音响连接/);
    await ui.tick({ device_connection: 'connected', transport_state: 'STOPPED' });
    expect(screen.getByRole('button', { name: '播放 第一首' })).toBeEnabled();
    expect(ui.calls('play_track')).toHaveLength(0);
    expect(ui.calls('control')).toHaveLength(0);
  });

  it('keeps one semantic play button per row and separates edit/remove from playback', async () => {
    const ui = setup();
    await ui.ready();
    const row = screen.getByRole('button', { name: '播放 第一首' }).closest('li');
    expect(within(row).getAllByRole('button', { name: '播放 第一首' })).toHaveLength(1);
    edit('第一首');
    expect(dialog()).toBeVisible();
    click('取消');
    click('移出曲库：第一首');
    const remove = screen.getByRole('dialog', { name: '移出这首音乐？' });
    expect(within(remove).getByText(/音频文件会保留/)).toBeTruthy();
    expect(within(remove).getByRole('button', { name: '取消' })).toHaveFocus();
    fireEvent.click(within(remove).getByRole('button', { name: '取消' }));
    expect(ui.calls('play_track')).toHaveLength(0);
    click('播放 第一首');
    await settle();
    expect(ui.calls('play_track')).toEqual([['play_track', { trackId: 'first' }]]);
    await ui.tick({ device_connection: 'disconnected' });
    expect(screen.getByRole('button', { name: '播放 第一首' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '移出曲库：第一首' })).toBeEnabled();
  });

  it('offers explicit discovery for an empty result without rediscovering from periodic reads', async () => {
    const ui = setup({
      initial: snapshot({ devices: [], selected_device_id: null, device_connection: 'unselected' }),
    });
    await ui.ready();
    await ui.tick();
    await ui.tick();
    expect(document.getElementById('device-hint').textContent).toMatch(/可重新搜索或手动添加/);
    expect(ui.calls('discover_devices')).toHaveLength(0);
    click('重新搜索音响');
    await settle();
    expect(ui.calls('discover_devices')).toHaveLength(1);
  });

  it('sends one atomic play-all command for the full library and explains queue boundaries', async () => {
    const ui = setup({ initial: snapshot({ current_track_id: 'first' }) });
    await ui.ready();
    expect(document.getElementById('queue-help').textContent).toMatch(/下一首：第二首/);
    fireEvent.change(screen.getByLabelText('搜索音乐'), { target: { value: '第二首' } });
    click('播放全部');
    await settle();
    expect(ui.calls('control')).toEqual([['control', { action: 'play-all', value: null }]]);
    expect(ui.calls('play_track')).toHaveLength(0);
    expect(ui.get().current_track_id).toBe('first');
    await ui.tick({ current_track_id: 'second' });
    expect(document.getElementById('queue-help').textContent).toMatch(/最后一首，播完停止/);
    await ui.tick({ queue_mode: 'repeat-all' });
    expect(document.getElementById('queue-help').textContent).toMatch(/下一首：第一首/);
  });

  it('keeps local search IME-safe, restores paging, clamps filtered results and returns focus after clear', async () => {
    const tracks = Array.from({ length: 25 }, (_, index) =>
      track(`song-${index}`, `歌曲 ${index + 1}`),
    );
    sessionStorage.setItem('music-library-view', JSON.stringify({ query: '', page: 2 }));
    const ui = setup({ initial: snapshot({ tracks }) });
    await ui.ready();
    expect(document.getElementById('page-info').textContent).toBe('2 / 2');
    expect(screen.getByRole('button', { name: '播放 歌曲 21' })).toBeVisible();
    const search = screen.getByLabelText('搜索音乐');
    fireEvent.compositionStart(search);
    fireEvent.change(search, { target: { value: '歌曲 3' } });
    expect(screen.getByRole('button', { name: '播放 歌曲 21' })).toBeVisible();
    fireEvent.compositionEnd(search);
    expect(screen.getByRole('button', { name: '播放 歌曲 3' })).toBeVisible();
    expect(document.getElementById('page-info').textContent).toBe('1 / 1');
    click('清空搜索');
    expect(search).toHaveFocus();
    expect(search.value).toBe('');
    expect(screen.getAllByRole('button', { name: /^播放 歌曲 / })).toHaveLength(20);
    expect(JSON.parse(sessionStorage.getItem('music-library-view'))).toEqual({
      query: '',
      page: 1,
    });
    expect(ui.calls('play_track')).toHaveLength(0);
  });

  it('keeps every rendered control ID unique across the desktop and persistent feature dialogs', async () => {
    const ui = setup();
    await ui.ready();
    edit('第一首');
    const ids = [...ui.container.querySelectorAll('[id]')].map((node) => node.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toContain('voice-dialog');
    expect(ids).toContain('bilibili-dialog');
    expect(ids).toContain('mobile-dialog');
    expect(ids).toContain('track-name-dialog');
  });
});

describe('React track naming and removal', () => {
  it('changes only the recognition draft, saves with expectedTitle and restores the row action focus', async () => {
    const ui = setup();
    await ui.ready();
    edit('第一首');
    expect(screen.getByLabelText('曲名').value).toBe('第一首');
    click('识别曲名');
    await settle();
    expect(screen.getByLabelText('曲名').value).toBe('晴天');
    expect(ui.get().tracks[0].title).toBe('第一首');
    expect(document.getElementById('track-name-recognition').textContent).toMatch(/音频标签/);
    expect(document.getElementById('track-name-voice').textContent).toMatch(/播放本地 晴天/);
    click('保存曲名');
    await settle();
    expect(ui.calls('rename_track')).toEqual([
      ['rename_track', { trackId: 'first', title: '晴天', expectedTitle: '第一首' }],
    ]);
    expect(screen.queryByRole('dialog', { name: '识别或修改曲名' })).toBeNull();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(screen.getByRole('button', { name: '识别或修改曲名：晴天' })).toHaveFocus();
    expect(document.getElementById('toast').textContent).toMatch(/播放本地 晴天/);
  });

  it('retains a cancelled draft and keeps it when recognition has no reliable result', async () => {
    const ui = setup({
      operation: (command) =>
        command === 'recognize_track_name' ? { title: null, source: 'filename' } : undefined,
    });
    await ui.ready();
    edit('第一首');
    changeName('自定曲名');
    click('识别曲名');
    await settle();
    expect(screen.getByLabelText('曲名').value).toBe('自定曲名');
    expect(document.getElementById('track-name-recognition').textContent).toMatch(/未找到可靠曲名/);
    click('取消');
    edit('第一首');
    expect(screen.getByLabelText('曲名').value).toBe('自定曲名');
    expect(ui.calls('rename_track')).toHaveLength(0);
  });

  it.each(['  ', '!!!', '歌'.repeat(121)])(
    'rejects an invalid title without changing the library (%s)',
    async (name) => {
      const ui = setup();
      await ui.ready();
      edit('第一首');
      changeName(name);
      click('保存曲名');
      await settle();
      const field = screen.getByLabelText('曲名');
      expect(field).toHaveAttribute('aria-invalid', 'true');
      expect(field).toHaveFocus();
      expect(document.getElementById('track-name-form-error').textContent).toMatch(/请输入/);
      expect(ui.calls('rename_track')).toHaveLength(0);
      expect(ui.get().tracks[0].title).toBe('第一首');
      expect(document.getElementById('toast').textContent).toBe('');
    },
  );

  it('blocks IME submission, duplicate saves and dismissal, then preserves the draft after a failed save', async () => {
    const pending = deferred();
    const ui = setup({
      operation: (command) => (command === 'rename_track' ? pending.promise : undefined),
    });
    await ui.ready();
    edit('第一首');
    changeName('新曲名');
    const field = screen.getByLabelText('曲名');
    const form = field.closest('form');
    fireEvent.compositionStart(field);
    fireEvent.submit(form);
    await settle();
    expect(ui.calls('rename_track')).toHaveLength(0);
    fireEvent.compositionEnd(field);
    fireEvent.submit(form);
    fireEvent.submit(form);
    await settle();
    expect(ui.calls('rename_track')).toHaveLength(1);
    expect(field).toBeDisabled();
    click('关闭识别或修改曲名');
    const cancel = new Event('cancel', { cancelable: true });
    fireEvent(dialog(), cancel);
    expect(cancel.defaultPrevented).toBe(true);
    expect(dialog().open).toBe(true);
    await act(async () => {
      pending.reject(new Error('保存失败，请检查磁盘空间'));
    });
    expect(dialog()).toBeVisible();
    expect(field).toBeEnabled();
    expect(field.value).toBe('新曲名');
    expect(field).toHaveFocus();
    expect(document.getElementById('track-name-form-error').textContent).toMatch(/保存失败/);
    expect(ui.get().tracks[0].title).toBe('第一首');
    expect(document.getElementById('toast').textContent).toBe('');
  });

  it('confirms removal, keeps source-file copy clear, clamps the last page and focuses a surviving row', async () => {
    const tracks = Array.from({ length: 21 }, (_, index) =>
      track(`song-${index}`, `歌曲 ${index + 1}`),
    );
    sessionStorage.setItem('music-library-view', JSON.stringify({ query: '', page: 2 }));
    const ui = setup({
      initial: snapshot({ tracks, current_track_id: 'song-20', transport_state: 'PLAYING' }),
    });
    await ui.ready();
    click('移出曲库：歌曲 21');
    const remove = screen.getByRole('dialog', { name: '移出这首音乐？' });
    expect(within(remove).getByText(/音频文件会保留/)).toBeTruthy();
    fireEvent.click(within(remove).getByRole('button', { name: '移出曲库', exact: true }));
    await settle();
    expect(ui.calls('remove_track')).toEqual([['remove_track', { trackId: 'song-20' }]]);
    expect(ui.get().current_track_id).toBeNull();
    expect(document.getElementById('page-info').textContent).toBe('1 / 1');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(screen.getByRole('button', { name: '移出曲库：歌曲 1' })).toHaveFocus();
    expect(document.getElementById('toast').textContent).toMatch(/音频文件已保留/);
    expect(ui.calls('play_track')).toHaveLength(0);
  });
});

describe('React native import and async state', () => {
  it('treats native file-picker cancellation honestly and renders the confirmed folder-import summary', async () => {
    const ui = setup({
      operation: (command, _args, ctx) => {
        if (command === 'pick_music_files') return null;
        if (command === 'pick_music_folder') {
          ctx.set({ tracks: [...ctx.get().tracks, track('third', '第三首')] });
          return { added: 1, total: 3, skipped: 1, warnings: ['跳过不支持的格式'] };
        }
        return undefined;
      },
    });
    await ui.ready();
    click('添加音乐');
    await settle();
    expect(document.getElementById('import-status')).toBeNull();
    expect(document.getElementById('toast').textContent).toBe('');
    click('导入文件夹');
    await settle();
    expect(document.getElementById('import-status-title').textContent).toBe('已加入 1 首音乐');
    expect(document.getElementById('import-status-copy').textContent).toMatch(
      /曲库共 3 首.*跳过 1 个文件.*跳过不支持的格式/,
    );
    expect(screen.getByRole('button', { name: '播放 第三首' })).toBeVisible();
    expect(ui.calls('pick_music_files')).toHaveLength(1);
    expect(ui.calls('pick_music_folder')).toHaveLength(1);
  });

  it('does not replay an outstanding native picker or unlock controls before it settles', async () => {
    const pending = deferred();
    const ui = setup({
      operation: (command) => (command === 'pick_music_files' ? pending.promise : undefined),
    });
    await ui.ready();
    const add = screen.getByRole('button', { name: '添加音乐' });
    fireEvent.click(add);
    fireEvent.click(add);
    expect(ui.calls('pick_music_files')).toHaveLength(1);
    expect(add).toBeDisabled();
    expect(screen.getByRole('button', { name: '播放 第一首' })).toBeDisabled();
    await act(async () => {
      pending.resolve(null);
    });
    expect(add).toBeEnabled();
    expect(document.getElementById('import-status')).toBeNull();
  });

  it('discards a poll started before a mutation instead of overwriting its confirmed state', async () => {
    const ui = setup();
    await ui.ready();
    const stale = ui.pauseNextRead();
    await ui.tick();
    click('播放 第二首');
    await settle();
    expect(document.getElementById('now-title').textContent).toBe('第二首');
    expect(document.getElementById('now-subtitle').textContent).toMatch(/正在播放/);
    await act(async () => {
      stale.resolve(snapshot({ current_track_id: 'first', transport_state: 'STOPPED' }));
    });
    expect(document.getElementById('now-title').textContent).toBe('第二首');
    expect(document.getElementById('now-subtitle').textContent).toMatch(/正在播放/);
    expect(ui.calls('play_track')).toEqual([['play_track', { trackId: 'second' }]]);
  });
});
