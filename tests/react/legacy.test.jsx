import React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import LegacyApp from '../../xiaomi_dlna/static/src/App.jsx';

const config = {
  formats: ['mp3', 'flac'],
  max_upload_bytes: 1024 * 1024,
  control_token: 'local-control',
};
const song = (id, title = `歌曲${id}`) => ({
  id: String(id),
  title,
  filename: `${id}.mp3`,
  format: 'mp3',
  size: 100,
});
const initial = {
  tracks: [song(1)],
  devices: [{ id: 'speaker', name: '小爱音箱', model: 'Pro' }],
  selected_device_id: 'speaker',
  current_track_id: null,
  transport_state: 'STOPPED',
  duration: 180,
  position: 0,
  volume: 30,
  queue_mode: 'sequential',
  warnings: [],
};
function service(respond) {
  return vi.fn(async (path, options) => {
    const supplied = await respond?.(path, options);
    const data = supplied ?? (path === '/api/config' ? config : initial);
    return { ok: !data.status, status: data.status || 200, json: async () => data };
  });
}
function uploads() {
  const instances = [];
  const makeXHR = () => {
    const instance = {
      headers: {},
      upload: {
        addEventListener(name, callback) {
          instance.progress = callback;
        },
      },
      open: vi.fn(),
      setRequestHeader(name, value) {
        this.headers[name] = value;
      },
      send: vi.fn(),
      abort: vi.fn(function () {
        this.onabort?.();
      }),
      complete(status = 200, data = { track: song('new') }) {
        this.status = status;
        this.responseText = JSON.stringify(data);
        this.onload();
      },
    };
    instances.push(instance);
    return instance;
  };
  return { instances, makeXHR };
}
beforeEach(() => history.replaceState(null, '', '/'));
afterEach(() => vi.restoreAllMocks());

describe('React Python browser UI', () => {
  it('loads the library and sends one authenticated playback mutation while pending', async () => {
    let settle;
    const fetcher = service((path) =>
      path === '/api/play'
        ? new Promise((resolve) => {
            settle = resolve;
          })
        : undefined,
    );
    render(<LegacyApp fetcher={fetcher} pollInterval={60000} />);
    await screen.findByText('本地服务已连接');
    const play = screen.getByRole('button', { name: '播放 歌曲1' });
    fireEvent.click(play);
    fireEvent.click(play);
    expect(play).toBeDisabled();
    const posts = fetcher.mock.calls.filter(([path]) => path === '/api/play');
    expect(posts).toHaveLength(1);
    expect(posts[0][1].headers['X-Control-Token']).toBe('local-control');
    expect(JSON.parse(posts[0][1].body)).toEqual({ track_id: '1' });
    await act(async () =>
      settle({ ...initial, current_track_id: '1', transport_state: 'PLAYING' }),
    );
    await screen.findByRole('button', { name: '暂停' });
  });

  it('keeps IME search pending, paginates and restores a local query in the URL', async () => {
    const fetcher = service((path) =>
      path === '/api/state'
        ? { ...initial, tracks: Array.from({ length: 26 }, (_, index) => song(index + 1)) }
        : undefined,
    );
    render(<LegacyApp fetcher={fetcher} pollInterval={60000} />);
    await screen.findByText('本地服务已连接');
    expect(
      within(screen.getByRole('list', { name: '音乐列表' })).getAllByRole('listitem'),
    ).toHaveLength(20);
    await userEvent.click(screen.getByRole('button', { name: '下一页' }));
    expect(location.search).toBe('?page=2');
    const search = screen.getByRole('searchbox', { name: '搜索音乐' });
    fireEvent.compositionStart(search);
    fireEvent.change(search, { target: { value: '歌曲25' } });
    expect(
      within(screen.getByRole('list', { name: '音乐列表' })).getAllByRole('listitem'),
    ).toHaveLength(6);
    fireEvent.compositionEnd(search, { target: { value: '歌曲25' } });
    expect(
      within(screen.getByRole('list', { name: '音乐列表' })).getAllByRole('listitem'),
    ).toHaveLength(1);
    expect(new URLSearchParams(location.search).get('q')).toBe('歌曲25');
    await userEvent.click(screen.getByRole('button', { name: '清空搜索' }));
    expect(search).toHaveFocus();
    expect(location.search).toBe('');
  });

  it('validates folder input and preserves its draft on a server error', async () => {
    const fetcher = service((path) =>
      path === '/api/library/scan' ? { status: 400, error: '文件夹不存在，请检查路径' } : undefined,
    );
    render(<LegacyApp fetcher={fetcher} pollInterval={60000} />);
    await screen.findByText('本地服务已连接');
    await userEvent.click(screen.getByRole('button', { name: '导入文件夹' }));
    const dialog = screen.getByRole('dialog');
    const field = within(dialog).getByRole('textbox', { name: '文件夹完整路径' });
    await userEvent.click(within(dialog).getByRole('button', { name: '导入文件夹' }));
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(field).toHaveFocus();
    await userEvent.type(field, '/music/missing');
    await userEvent.click(within(dialog).getByRole('button', { name: '导入文件夹' }));
    await within(dialog).findByText('文件夹不存在，请检查路径');
    expect(field).toHaveValue('/music/missing');
    expect(field).toHaveFocus();
    expect(
      JSON.parse(fetcher.mock.calls.find(([path]) => path === '/api/library/scan')[1].body),
    ).toEqual({ path: '/music/missing' });
  });

  it('refreshes an expired control token without replaying the failed command', async () => {
    let configs = 0;
    const fetcher = service((path) =>
      path === '/api/config'
        ? { ...config, control_token: ++configs === 1 ? 'old' : 'fresh' }
        : path === '/api/play'
          ? { status: 403, error: '控制令牌已更新，请重试' }
          : undefined,
    );
    render(<LegacyApp fetcher={fetcher} pollInterval={60000} />);
    await screen.findByText('本地服务已连接');
    await userEvent.click(screen.getByRole('button', { name: '播放 歌曲1' }));
    await screen.findByText('控制令牌已更新，请重试');
    expect(configs).toBe(2);
    expect(fetcher.mock.calls.filter(([path]) => path === '/api/play')).toHaveLength(1);
    await userEvent.click(screen.getByRole('button', { name: '播放 歌曲1' }));
    const calls = fetcher.mock.calls.filter(([path]) => path === '/api/play');
    expect(calls[1][1].headers['X-Control-Token']).toBe('fresh');
  });

  it('preserves a focused slider during polling and reconciles the latest value on blur without sending a command', async () => {
    let current = initial;
    const fetcher = service((path) => (path === '/api/state' ? current : undefined));
    render(<LegacyApp fetcher={fetcher} pollInterval={30} />);
    await screen.findByText('本地服务已连接');
    const volume = screen.getByRole('slider', { name: '音量' });
    act(() => volume.focus());
    current = { ...initial, volume: 65, warnings: ['已同步音响状态'] };
    await screen.findByText('已同步音响状态');
    expect(volume).toHaveValue('30');
    fireEvent.blur(volume);
    await waitFor(() => expect(volume).toHaveValue('65'));
    expect(fetcher.mock.calls.filter(([path]) => path === '/api/control')).toHaveLength(0);
  });

  it('reconciles a failed slider command when the authoritative value is unchanged and never commits it twice', async () => {
    let settle;
    const fetcher = service((path) =>
      path === '/api/control'
        ? new Promise((resolve) => {
            settle = resolve;
          })
        : undefined,
    );
    render(<LegacyApp fetcher={fetcher} pollInterval={60000} />);
    await screen.findByText('本地服务已连接');
    const volume = screen.getByRole('slider', { name: '音量' });
    act(() => volume.focus());
    fireEvent.change(volume, { target: { value: '65' } });
    fireEvent.pointerUp(volume);
    fireEvent.blur(volume);
    expect(volume).toHaveValue('65');
    expect(screen.getByText('65', { exact: true })).toBeInTheDocument();
    expect(fetcher.mock.calls.filter(([path]) => path === '/api/control')).toHaveLength(1);
    await act(async () => settle({ status: 500, error: '音响拒绝调整音量' }));
    await screen.findByText('音响拒绝调整音量');
    await waitFor(() => expect(volume).toHaveValue('30'));
    expect(screen.getByText('30', { exact: true })).toBeInTheDocument();
    fireEvent.keyUp(volume, { key: 'ArrowRight' });
    expect(fetcher.mock.calls.filter(([path]) => path === '/api/control')).toHaveLength(1);
  });

  it('uploads sequentially with progress, cancellation, explicit retry and validation', async () => {
    const transport = uploads();
    render(<LegacyApp fetcher={service()} makeXHR={transport.makeXHR} pollInterval={60000} />);
    await screen.findByText('本地服务已连接');
    const input = screen.getByLabelText('选择本地音乐文件');
    const files = [
      new File(['audio'], '第一首.mp3'),
      new File(['audio'], '第二首.mp3'),
      new File(['text'], 'notes.txt'),
    ];
    fireEvent.change(input, { target: { files } });
    await waitFor(() => expect(transport.instances).toHaveLength(1));
    expect(transport.instances[0].headers['X-Filename']).toBe(encodeURIComponent('第一首.mp3'));
    act(() => transport.instances[0].progress({ lengthComputable: true, loaded: 2, total: 4 }));
    expect(screen.getByRole('progressbar', { name: '第一首.mp3 上传进度' })).toHaveValue(50);
    act(() => transport.instances[0].complete());
    await waitFor(() => expect(transport.instances).toHaveLength(2));
    await userEvent.click(screen.getByRole('button', { name: '取消 第二首.mp3' }));
    expect(transport.instances[1].abort).toHaveBeenCalledTimes(1);
    const retry = await screen.findByRole('button', { name: '重试 第二首.mp3' });
    await userEvent.click(retry);
    await waitFor(() => expect(transport.instances).toHaveLength(3));
    act(() => transport.instances[2].complete(400, { error: '上传未完成，请重试' }));
    await screen.findByText(/上传未完成，请重试/);
    await userEvent.click(screen.getByRole('button', { name: '移除 notes.txt' }));
    expect(screen.queryByText('notes.txt')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: '重试 第二首.mp3' })).toBeEnabled();
  });
});
