import React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import MobileApp from '../../mobile/src/MobileApp.jsx';

const SESSION_KEY = 'mobile-session-v1';
const player = {
  mode: 'sequential',
  connection: 'connected',
  device_id: 'speaker',
  device_name: '小爱 Pro',
  current_track_id: null,
  transport: 'STOPPED',
};
const library = {
  player,
  tracks: [{ id: 'song', title: '晴天', format: 'm4a' }],
  total: 1,
  library_total: 1,
  page: 1,
  pages: 1,
};

function phone({ stored = '', ticket = '', reply, storageFails = false } = {}) {
  if (stored) localStorage.setItem(SESSION_KEY, stored);
  history.replaceState(null, '', ticket ? `/#pair=${ticket}` : '/');
  if (storageFails)
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw Error('disabled');
    });
  let respond = reply || (() => ({ token: 'new-phone-token', jobs: [] }));
  const calls = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url, init) => {
      const call = {
        url,
        body: init.body ? JSON.parse(init.body) : undefined,
        headers: init.headers,
        hash: location.hash,
      };
      calls.push(call);
      const raw = await respond(url, init);
      const data = url.startsWith('/api/library') && !raw.status ? { ...library, ...raw } : raw;
      return { ok: !data.status, status: data.status || 200, json: async () => data };
    }),
  );
  const ui = render(<MobileApp />);
  return {
    ...ui,
    calls,
    respond: (next) => {
      respond = next;
    },
    user: userEvent.setup(),
  };
}
const ready = () =>
  waitFor(() => expect(screen.getByRole('button', { name: '播放 晴天' }).disabled).toBe(false));
async function poll(ui) {
  const before = ui.calls.filter((call) => call.url === '/api/jobs').length;
  fireEvent(document, new Event('visibilitychange'));
  await waitFor(() =>
    expect(ui.calls.filter((call) => call.url === '/api/jobs').length).toBeGreaterThan(before),
  );
}

beforeEach(() => {
  localStorage.clear();
  history.replaceState(null, '', '/');
  Object.defineProperty(document, 'hidden', { value: false, configurable: true });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('React phone pairing and playback', () => {
  it('removes the QR ticket before the automatic request and remembers only the bearer token', async () => {
    const ui = phone({ ticket: 'one-use-ticket' });
    await ready();
    expect(ui.calls[0]).toMatchObject({
      url: '/api/pair',
      body: { ticket: 'one-use-ticket' },
      hash: '',
    });
    expect(localStorage.getItem(SESSION_KEY)).toBe('new-phone-token');
    expect(localStorage.length).toBe(1);
    expect(screen.queryByRole('heading', { name: '连接电脑' })).toBeNull();
    expect(ui.calls.every((call) => !call.url.includes('one-use-ticket'))).toBe(true);
  });

  it.each(['', 'used-ticket'])(
    'restores saved pairing without redeeming the QR again (%s)',
    async (ticket) => {
      const ui = phone({ stored: 'saved-token', ticket });
      await ready();
      expect(ui.calls[0].headers.Authorization).toBe('Bearer saved-token');
      expect(
        ui.calls.every((call) => call.url === '/api/jobs' || call.url.startsWith('/api/library')),
      ).toBe(true);
      expect(screen.getByRole('heading', { name: '选择视频音轨' })).toBeTruthy();
    },
  );

  it('retains an offline saved credential, hides connected controls, and reconnects without another QR', async () => {
    const ui = phone({
      stored: 'saved-token',
      reply: () => {
        throw new TypeError('network');
      },
    });
    await screen.findByText(/暂时无法连接电脑/);
    expect(localStorage.getItem(SESSION_KEY)).toBe('saved-token');
    expect(screen.queryByRole('heading', { name: '选择视频音轨' })).toBeNull();
    ui.respond(() => ({ jobs: [] }));
    await ui.user.click(screen.getByRole('button', { name: '重试连接' }));
    await ready();
    expect(document.getElementById('connection-error').textContent).toBe('');
    expect(ui.calls.some((call) => call.url === '/api/pair')).toBe(false);
  });

  it('revokes an old bearer and immediately uses the newly scanned ticket', async () => {
    const ui = phone({
      stored: 'old',
      ticket: 'fresh',
      reply: (_url, init) =>
        init.headers.Authorization === 'Bearer old'
          ? { status: 401, error: '已忘记手机' }
          : { token: 'fresh-token', jobs: [] },
    });
    await ready();
    expect(localStorage.getItem(SESSION_KEY)).toBe('fresh-token');
    expect(ui.calls.filter((call) => call.url === '/api/pair')).toHaveLength(1);
  });

  it('keeps an expired-ticket error visible without exposing connected controls', async () => {
    phone({ ticket: 'expired', reply: () => ({ status: 403, error: '二维码已过期' }) });
    await screen.findByText('二维码已过期');
    expect(screen.queryByRole('heading', { name: '选择视频音轨' })).toBeNull();
    expect(localStorage.length).toBe(0);
  });

  it('supports in-memory pairing when storage is blocked and explains the limitation', async () => {
    phone({ ticket: 'ticket', storageFails: true });
    await ready();
    expect(localStorage.length).toBe(0);
    expect(screen.getByText(/浏览器未允许保存配对/)).toBeTruthy();
  });

  it('retains a draft through a temporary disconnect and hides controls after revocation', async () => {
    const ui = phone({ stored: 'saved' });
    await ready();
    await ui.user.type(screen.getByLabelText('视频链接或分享文案'), 'my draft');
    ui.respond(() => {
      throw new TypeError('offline');
    });
    await poll(ui);
    await screen.findByText(/暂时无法连接电脑/);
    expect(screen.getByLabelText('视频链接或分享文案').value).toBe('my draft');
    expect(localStorage.getItem(SESSION_KEY)).toBe('saved');
    expect(screen.getByRole('button', { name: '播放 晴天' }).disabled).toBe(true);
    ui.respond(() => ({ status: 401, error: '请重新扫码' }));
    await poll(ui);
    await screen.findByText(/配对已失效/);
    expect(localStorage.getItem(SESSION_KEY)).toBeNull();
    expect(screen.queryByRole('heading', { name: '选择视频音轨' })).toBeNull();
  });

  it('dispatches a chosen track and an explicit mode change once with the required speaker boundary', async () => {
    const ui = phone({
      stored: 'paired',
      reply: (url, init) =>
        url === '/api/playback'
          ? {
              ...player,
              ...(JSON.parse(init.body).action === 'mode'
                ? { mode: JSON.parse(init.body).mode }
                : { current_track_id: 'song', title: '晴天', transport: 'TRANSITIONING' }),
            }
          : { jobs: [] },
    });
    await ready();
    await ui.user.click(screen.getByRole('button', { name: '播放 晴天' }));
    await waitFor(() => expect(screen.getByLabelText('播放模式').disabled).toBe(false));
    await ui.user.selectOptions(screen.getByLabelText('播放模式'), 'repeat-all');
    await ui.user.click(screen.getByRole('button', { name: '应用播放模式' }));
    await waitFor(() =>
      expect(ui.calls.filter((call) => call.url === '/api/playback')).toHaveLength(2),
    );
    const requests = ui.calls.filter((call) => call.url === '/api/playback');
    expect(requests[0].body).toEqual({ action: 'play', track_id: 'song', device_id: 'speaker' });
    expect(requests[1].body).toEqual({ action: 'mode', mode: 'repeat-all' });
  });

  it('disables stale playback controls and never mutates playback during polling', async () => {
    const ui = phone({ stored: 'paired' });
    await ready();
    ui.respond(() => {
      throw new TypeError('offline');
    });
    await poll(ui);
    await screen.findByText(/暂时无法连接电脑/);
    expect(screen.getByLabelText('播放模式').disabled).toBe(true);
    expect(screen.getByRole('button', { name: '下一首' }).disabled).toBe(true);
    expect(ui.calls.filter((call) => call.url === '/api/playback')).toHaveLength(0);
  });

  it('locks pending playback against duplicates and preserves an unapplied mode across polls', async () => {
    let complete;
    const ui = phone({
      stored: 'paired',
      reply: (url) =>
        url === '/api/playback'
          ? new Promise((resolve) => {
              complete = resolve;
            })
          : { jobs: [] },
    });
    await ready();
    await ui.user.selectOptions(screen.getByLabelText('播放模式'), 'repeat-one');
    await poll(ui);
    await waitFor(() => expect(screen.getByLabelText('播放模式').value).toBe('repeat-one'));
    const playAll = screen.getByRole('button', { name: '播放全部 · 从第一首顺序播放' });
    await ui.user.click(playAll);
    fireEvent.click(playAll);
    expect(ui.calls.filter((call) => call.url === '/api/playback')).toHaveLength(1);
    expect(playAll.getAttribute('aria-busy')).toBe('true');
    await act(async () => {
      complete({ ...player, transport: 'PLAYING' });
    });
    await waitFor(() => expect(playAll.getAttribute('aria-busy')).toBe('false'));
    expect(screen.getByLabelText('播放模式').value).toBe('repeat-one');
  });

  it('keeps IME search private, discards an obsolete response, and clears immediately', async () => {
    let resolveOld;
    const ui = phone({
      stored: 'paired',
      reply: (url) =>
        url.includes('q=old')
          ? new Promise((resolve) => {
              resolveOld = resolve;
            })
          : {
              jobs: [],
              ...(url.includes('q=new')
                ? { tracks: [{ id: 'new', title: '新歌', format: 'mp3' }] }
                : {}),
            },
    });
    await ready();
    const search = screen.getByLabelText('搜索曲名');
    fireEvent.compositionStart(search);
    fireEvent.change(search, { target: { value: 'old' } });
    await new Promise((resolve) => setTimeout(resolve, 330));
    expect(ui.calls.some((call) => call.url.includes('q=old'))).toBe(false);
    fireEvent.compositionEnd(search, { data: 'old' });
    await waitFor(() => expect(resolveOld).toBeTypeOf('function'));
    fireEvent.change(search, { target: { value: 'new' } });
    await screen.findByRole('button', { name: '播放 新歌' });
    await act(async () => {
      resolveOld({ tracks: [{ id: 'old', title: '旧歌', format: 'mp3' }] });
    });
    expect(screen.queryByRole('button', { name: '播放 旧歌' })).toBeNull();
    await ui.user.click(screen.getByRole('button', { name: '清空搜索曲名' }));
    await screen.findByRole('button', { name: '播放 晴天' });
    expect(search.value).toBe('');
    expect(document.activeElement).toBe(search);
    expect(location.search).toBe('');
  });

  it('clears share text and its validation without submitting, then restores input focus', async () => {
    const ui = phone({ stored: 'paired' });
    await ready();
    const field = screen.getByLabelText('视频链接或分享文案');
    expect(screen.queryByRole('button', { name: '清空视频链接或分享文案' })).toBeNull();
    await ui.user.click(screen.getByRole('button', { name: '发送并解析' }));
    await screen.findByText('请先粘贴 Bilibili 视频分享链接。');
    await waitFor(() => expect(field).toBeEnabled());
    await ui.user.type(field, 'BV1234567890');
    await ui.user.click(screen.getByRole('button', { name: '清空视频链接或分享文案' }));
    expect(field).toHaveValue('');
    expect(field).toHaveFocus();
    expect(field).toHaveAttribute('aria-invalid', 'false');
    expect(screen.queryByText('请先粘贴 Bilibili 视频分享链接。')).toBeNull();
    expect(screen.queryByRole('button', { name: '清空视频链接或分享文案' })).toBeNull();
    expect(ui.calls.filter((call) => call.url === '/api/parse')).toHaveLength(0);
  });

  it('clears the unsubmitted preview and selections while keeping existing downloads', async () => {
    const ui = phone({
      stored: 'paired',
      reply: (url) =>
        url === '/api/parse'
          ? {
              id: 'preview',
              title: '已解析视频',
              author: '作者',
              selected_page: 1,
              parts: [{ page: 1, cid: 1, title: '音轨' }],
            }
          : {
              jobs: [
                { id: 'existing', title: '已在下载的音乐', phase: 'downloading', downloaded: 0 },
              ],
            },
    });
    await ready();
    const field = screen.getByLabelText('视频链接或分享文案');
    await ui.user.type(field, 'BV1234567890');
    await ui.user.click(screen.getByRole('button', { name: '发送并解析' }));
    await screen.findByRole('heading', { name: '已解析视频' });
    await waitFor(() => expect(field).toBeEnabled());
    await ui.user.click(screen.getByRole('button', { name: '清空视频链接或分享文案' }));
    expect(field).toHaveValue('');
    expect(field).toHaveFocus();
    expect(screen.queryByRole('heading', { name: '已解析视频' })).toBeNull();
    expect(screen.queryByRole('checkbox', { name: 'P1 · 音轨' })).toBeNull();
    expect(screen.getByText('已在下载的音乐')).toBeVisible();
    expect(ui.calls.filter((call) => call.url === '/api/download')).toHaveLength(0);
  });

  it('disables clearing during parsing and enables it after the original request finishes', async () => {
    let finish;
    const ui = phone({
      stored: 'paired',
      reply: (url) =>
        url === '/api/parse'
          ? new Promise((resolve) => {
              finish = resolve;
            })
          : { jobs: [] },
    });
    await ready();
    const field = screen.getByLabelText('视频链接或分享文案');
    await ui.user.type(field, 'BV1234567890');
    await ui.user.click(screen.getByRole('button', { name: '发送并解析' }));
    const clear = screen.getByRole('button', { name: '清空视频链接或分享文案' });
    expect(clear).toBeDisabled();
    fireEvent.click(clear);
    expect(field).toHaveValue('BV1234567890');
    await act(async () => {
      finish({
        id: 'preview',
        title: '测试视频',
        author: '作者',
        selected_page: 1,
        parts: [{ page: 1, cid: 1, title: '音轨' }],
      });
    });
    await waitFor(() => expect(clear).toBeEnabled());
    await ui.user.click(clear);
    expect(field).toHaveValue('');
    expect(field).toHaveFocus();
    expect(ui.calls.filter((call) => call.url === '/api/parse')).toHaveLength(1);
  });

  it('selects only the linked part, retains rejected downloads and preserves a draft on parse failure', async () => {
    const preview = {
      id: 'preview',
      title: '我的视频',
      author: '作者',
      selected_page: 2,
      parts: [
        { page: 1, cid: 1, title: '第一首' },
        { page: 2, cid: 2, title: '第二首' },
      ],
    };
    const ui = phone({
      stored: 'paired',
      reply: (url) =>
        url === '/api/parse'
          ? preview
          : url === '/api/download'
            ? {
                items: [
                  { cid: 1, title: '第一首', accepted: false, error: '已有歌曲' },
                  { cid: 2, title: '第二首', accepted: true },
                ],
              }
            : { jobs: [] },
    });
    await ready();
    await ui.user.type(screen.getByLabelText('视频链接或分享文案'), 'BV1234567890?p=2');
    await ui.user.click(screen.getByRole('button', { name: '发送并解析' }));
    const first = await screen.findByLabelText('P1 · 第一首');
    const second = screen.getByLabelText('P2 · 第二首');
    expect(first.checked).toBe(false);
    expect(second.checked).toBe(true);
    await ui.user.click(first);
    await ui.user.click(screen.getByRole('button', { name: '在电脑下载选中音轨' }));
    await screen.findByText('已发送 1 首，电脑将下载并加入曲库。');
    expect(first.checked).toBe(true);
    expect(second.checked).toBe(false);
    expect(screen.getByText('第一首：已有歌曲')).toBeTruthy();
    expect(ui.calls.find((call) => call.url === '/api/download').body).toEqual({
      preview_id: 'preview',
      cids: [1, 2],
    });
    ui.respond((url) =>
      url === '/api/parse' ? { status: 403, error: '请在电脑登录 Bilibili' } : { jobs: [] },
    );
    await ui.user.click(screen.getByRole('button', { name: '发送并解析' }));
    await screen.findByText('请在电脑登录 Bilibili');
    expect(screen.getByLabelText('视频链接或分享文案').value).toBe('BV1234567890?p=2');
  });
});
