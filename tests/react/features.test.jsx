import React from 'react';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import FeaturePanels from '../../frontend/src/features/FeaturePanels.jsx';

const snapshot = () => ({
  voice: { connected: false, listening: false, devices: [] },
  downloads: {
    authenticated: false,
    directory: '/fixture/downloads',
    jobs: [],
  },
  xiaomi_login: { phase: 'idle' },
  bilibili_login: { phase: 'idle' },
  selected_device_id: 'speaker',
  selected_device_name: '小爱 Pro',
});

function setup({ panel = 'voice', initial = snapshot(), operation } = {}) {
  let data = initial;
  const invoke = vi.fn(async (command, args) =>
    command === 'get_features' ? data : operation?.(command, args),
  );
  const announce = vi.fn();
  const onVoiceBadge = vi.fn();
  const onClose = vi.fn();
  const refresh = vi.fn();
  const queueTarget = document.createElement('div');
  document.body.append(queueTarget);
  const props = {
    panel,
    onClose,
    invoke,
    connected: true,
    refresh,
    announce,
    onVoiceBadge,
    queueTarget,
  };
  const view = render(<FeaturePanels {...props} />);
  return {
    ...view,
    invoke,
    announce,
    onVoiceBadge,
    onClose,
    refresh,
    queueTarget,
    async tick(next) {
      if (next) data = next;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
    },
    async ready() {
      await act(async () => {
        await Promise.resolve();
      });
    },
    switchPanel(next) {
      view.rerender(<FeaturePanels {...props} panel={next} />);
    },
  };
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  document.body.replaceChildren();
  vi.useRealTimers();
});

describe('React account state', () => {
  it('shows waiting, checking and connected state and announces one successful transition', async () => {
    const data = snapshot();
    data.xiaomi_login.phase = 'waiting';
    const ui = setup({ initial: data });
    await ui.ready();
    expect(screen.getByText(/成功后窗口会自动关闭/)).toBeTruthy();
    data.xiaomi_login = { phase: 'checking' };
    await ui.tick({ ...data });
    expect(screen.getByRole('button', { name: '正在确认…' }).disabled).toBe(true);
    data.voice = { ...data.voice, connected: true };
    data.xiaomi_login = { phase: 'connected', cached: true };
    await ui.tick({ ...data });
    expect(screen.getByText('已连接 · 重启后自动恢复登录')).toBeTruthy();
    expect(screen.queryByRole('button', { name: '重试连接' })).toBeNull();
    expect(ui.onVoiceBadge).toHaveBeenLastCalledWith('已连接');
    await ui.tick();
    expect(
      ui.announce.mock.calls.filter(([message]) => message.includes('已连接小米账号')),
    ).toHaveLength(1);
  });

  it.each(['xiaomi', 'bilibili'])(
    'preserves cached but offline %s recovery state until validation succeeds',
    async (provider) => {
      const data = snapshot();
      data[`${provider}_login`] = { phase: 'checking', cached: true };
      const ui = setup({
        panel: provider === 'xiaomi' ? 'voice' : 'bilibili',
        initial: data,
      });
      await ui.ready();
      expect(document.getElementById(`${provider}-account-status`).textContent).toContain(
        '正在恢复',
      );
      data[`${provider}_login`] = {
        phase: 'failed',
        cached: true,
        error: '网络暂不可用',
      };
      await ui.tick({ ...data });
      expect(document.getElementById(`${provider}-account-status`).textContent).toContain(
        '暂未连接',
      );
      expect(document.getElementById(`${provider}-login-error`).textContent).toContain(
        '网络暂不可用',
      );
      data[provider === 'xiaomi' ? 'voice' : 'downloads'] = {
        ...data[provider === 'xiaomi' ? 'voice' : 'downloads'],
        [provider === 'xiaomi' ? 'connected' : 'authenticated']: true,
      };
      data[`${provider}_login`] = { phase: 'connected', cached: true };
      await ui.tick({ ...data });
      expect(document.getElementById(`${provider}-account-status`).textContent).toContain(
        '重启后自动恢复',
      );
      expect(document.getElementById(`${provider}-login-error`).textContent).toBe('');
    },
  );

  it('clears waiting controls after cancellation and keeps failed provider errors until retry', async () => {
    const data = snapshot();
    data.xiaomi_login = { phase: 'waiting' };
    const ui = setup({ initial: data });
    await ui.ready();
    data.xiaomi_login = { phase: 'cancelled' };
    await ui.tick({ ...data });
    expect(screen.getByText('登录已取消，可重新打开登录窗口。')).toBeTruthy();
    expect(screen.queryByRole('button', { name: '重试连接' })).toBeNull();
    data.xiaomi_login = { phase: 'failed', error: '连接失败，请检查网络' };
    await ui.tick({ ...data });
    expect(screen.getByText('连接失败，请检查网络')).toBeTruthy();
    expect(screen.getByRole('button', { name: '重试连接' }).disabled).toBe(false);
    await ui.tick();
    expect(screen.getByText('连接失败，请检查网络')).toBeTruthy();
  });
});

describe('React Bilibili import', () => {
  it('selects the linked part, submits selected CIDs and retains skipped tracks for correction', async () => {
    const preview = {
      id: 'preview',
      bvid: 'BVfixture',
      title: '现场音乐',
      author: '歌手',
      selected_page: 2,
      parts: [
        { cid: 10, page: 1, title: '第一首', duration: 10 },
        { cid: 20, page: 2, title: '第二首', duration: 20 },
      ],
    };
    const ui = setup({
      panel: 'bilibili',
      operation: (command) =>
        command === 'parse_bilibili'
          ? preview
          : command === 'download_bilibili_batch'
            ? {
                items: [
                  {
                    preview_id: 'preview',
                    cid: 20,
                    title: '第二首',
                    error: '队列已满',
                  },
                ],
              }
            : undefined,
    });
    await ui.ready();
    fireEvent.change(screen.getByLabelText('视频链接或 BV 号'), {
      target: { value: 'BVfixture' },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '解析视频' }));
    });
    const second = screen.getByRole('checkbox', { name: /P2 · 第二首/ });
    expect(second.checked).toBe(true);
    expect(screen.getByRole('checkbox', { name: /P1 · 第一首/ }).checked).toBe(false);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '下载选中音轨' }));
    });
    expect(ui.invoke).toHaveBeenCalledWith('download_bilibili_batch', {
      selections: [{ preview_id: 'preview', cids: [20] }],
    });
    expect(screen.getByText('第二首：队列已满')).toBeTruthy();
    expect(second.checked).toBe(true);
  });

  it('prevents duplicate parsing and closing while pending; IME submission never dispatches', async () => {
    let resolve;
    const ui = setup({
      panel: 'bilibili',
      operation: () =>
        new Promise((done) => {
          resolve = done;
        }),
    });
    await ui.ready();
    const field = screen.getByLabelText('视频链接或 BV 号');
    fireEvent.change(field, { target: { value: 'BVfixture' } });
    fireEvent.compositionStart(field);
    fireEvent.submit(field.closest('form'));
    expect(ui.invoke.mock.calls.filter(([command]) => command === 'parse_bilibili')).toHaveLength(
      0,
    );
    fireEvent.compositionEnd(field);
    await act(async () => {
      fireEvent.submit(field.closest('form'));
      fireEvent.submit(field.closest('form'));
    });
    expect(ui.invoke.mock.calls.filter(([command]) => command === 'parse_bilibili')).toHaveLength(
      1,
    );
    expect(screen.getByRole('button', { name: '解析视频' }).disabled).toBe(true);
    const dialog = field.closest('dialog');
    const event = new Event('cancel', { cancelable: true });
    fireEvent(dialog, event);
    expect(event.defaultPrevented).toBe(true);
    await act(async () => {
      resolve({
        id: 'preview',
        bvid: 'BVfixture',
        title: '歌曲',
        parts: [{ page: 1, cid: 1, title: '歌曲', duration: 1 }],
        selected_page: 1,
      });
    });
    expect(screen.getByRole('button', { name: '解析视频' }).disabled).toBe(false);
  });

  it('keeps invalid input and focuses the field with an inline error', async () => {
    const ui = setup({ panel: 'bilibili' });
    await ui.ready();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '解析视频' }));
    });
    expect(screen.getByText('请先粘贴视频链接或 BV 号。')).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByLabelText('视频链接或 BV 号'));
    expect(ui.invoke.mock.calls.filter(([command]) => command === 'parse_bilibili')).toHaveLength(
      0,
    );
  });
});

it('keeps download progress outside dialogs and retries only after an explicit action', async () => {
  const data = snapshot();
  data.downloads.jobs = [
    {
      id: 'job',
      title: '晴天',
      phase: 'failed',
      error: '网络错误',
      downloaded: 0,
    },
  ];
  const ui = setup({ panel: null, initial: data });
  await ui.ready();
  expect(within(ui.queueTarget).getByText('晴天')).toBeTruthy();
  expect(ui.invoke.mock.calls.filter(([command]) => command === 'retry_download')).toHaveLength(0);
  await act(async () => {
    fireEvent.click(within(ui.queueTarget).getByRole('button', { name: '重试 晴天' }));
  });
  expect(ui.invoke).toHaveBeenCalledWith('retry_download', { id: 'job' });
});

it('receiver opens paused and enables locally generated QR only on explicit request', async () => {
  const ui = setup({
    panel: 'mobile',
    operation: (() => {
      let enabled = false;
      return (command) => {
        if (command === 'set_mobile_receiver') enabled = true;
        return {
          enabled,
          paired: false,
          pairing_available: enabled,
          entries: enabled
            ? [
                {
                  address: '192.168.1.2',
                  url: 'http://192.168.1.2:8998/#pair=fixture',
                },
              ]
            : [],
        };
      };
    })(),
  });
  await ui.ready();
  expect(
    ui.invoke.mock.calls.filter(([command]) => command === 'set_mobile_receiver'),
  ).toHaveLength(0);
  expect(screen.queryByAltText('用手机扫码打开手机点歌页面')).toBeNull();
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '开启手机接收' }));
  });
  expect(ui.invoke).toHaveBeenCalledWith('set_mobile_receiver', {
    enabled: true,
  });
  expect(screen.getByAltText('用手机扫码打开手机点歌页面').getAttribute('src')).toMatch(
    /^data:image\/gif;base64,/,
  );
});

it('an older feature read cannot restore account state after a foreground account action', async () => {
  let finishOldRead;
  const current = snapshot();
  let first = true;
  const invoke = vi.fn(async (command) => {
    if (command !== 'get_features') return;
    if (first) {
      first = false;
      return new Promise((resolve) => {
        finishOldRead = resolve;
      });
    }
    return current;
  });
  const announce = vi.fn();
  render(
    <FeaturePanels
      panel="voice"
      onClose={vi.fn()}
      invoke={invoke}
      connected
      refresh={vi.fn()}
      announce={announce}
      onVoiceBadge={vi.fn()}
    />,
  );
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '登录小米账号' }));
  });
  const obsolete = snapshot();
  obsolete.voice.connected = true;
  obsolete.xiaomi_login.phase = 'connected';
  await act(async () => {
    finishOldRead(obsolete);
  });
  expect(screen.queryByText(/已连接 ·/)).toBeNull();
  expect(announce).not.toHaveBeenCalled();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
  expect(screen.getByText('未连接 · 使用绑定音响的小米账号')).toBeTruthy();
});

it('favorites paging preserves selected parts across pages and sends one explicit batch', async () => {
  const previews = Object.fromEntries(
    ['first', 'second'].map((bvid, index) => [
      bvid,
      {
        id: `preview-${bvid}`,
        bvid,
        title: bvid,
        author: '歌手',
        selected_page: 1,
        parts: [{ cid: index + 10, page: 1, title: bvid, duration: 60 }],
      },
    ]),
  );
  const ui = setup({
    panel: 'bilibili',
    operation: (command, args) => {
      if (command === 'read_bilibili_favorite')
        return {
          id: 'folder',
          title: '我的音乐',
          owner: '歌手',
          total: 2,
          page: args.page,
          has_more: args.page === 1,
          videos: [
            {
              bvid: args.page === 1 ? 'first' : 'second',
              title: args.page === 1 ? '第一首' : '第二首',
              author: '歌手',
              parts: 1,
              duration: 60,
            },
          ],
        };
      if (command === 'parse_bilibili') return previews[args.url];
      if (command === 'download_bilibili_batch')
        return {
          items: args.selections.flatMap((item) =>
            item.cids.map((cid) => ({
              preview_id: item.preview_id,
              cid,
              job: { id: `job-${cid}` },
            })),
          ),
        };
    },
  });
  await ui.ready();
  fireEvent.click(screen.getByRole('radio', { name: '收藏夹' }));
  fireEvent.change(screen.getByLabelText('收藏夹链接或 ID'), {
    target: { value: 'folder' },
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '读取收藏夹' }));
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '选择音轨：第一首' }));
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('checkbox', { name: /P1 · first/ }));
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '下一页' }));
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '选择音轨：第二首' }));
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('checkbox', { name: /P1 · second/ }));
  });
  expect(screen.getByText('已选 2 首 · 包含其他页的选择')).toBeTruthy();
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '下载选中音轨' }));
  });
  expect(ui.invoke).toHaveBeenCalledWith('download_bilibili_batch', {
    selections: [
      { preview_id: 'preview-first', cids: [10] },
      { preview_id: 'preview-second', cids: [11] },
    ],
  });
  expect(screen.getByText('已选 0 首 · 包含其他页的选择')).toBeTruthy();
});

it('advanced voice session stays masked and removes submitted cookies even when validation fails', async () => {
  const ui = setup({
    operation: () => {
      throw new Error('会话已失效，请重新登录');
    },
  });
  await ui.ready();
  fireEvent.click(screen.getByText('登录遇到问题？使用已有会话'));
  const field = screen.getByLabelText('小爱服务 Cookie');
  expect(field.type).toBe('password');
  fireEvent.change(field, {
    target: { value: 'userId=fixture; serviceToken=fixture' },
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '连接会话' }));
  });
  expect(ui.invoke).toHaveBeenCalledWith('connect_voice_session', {
    cookie: 'userId=fixture; serviceToken=fixture',
  });
  expect(field.value).toBe('');
  expect(screen.getByText('会话已失效，请重新登录')).toBeTruthy();
});

it('voice binding uses the selected cloud source and current desktop target only after confirmation', async () => {
  const data = snapshot();
  data.voice = {
    ...data.voice,
    connected: true,
    devices: [
      { id: 'cloud-one', name: '客厅', hardware: 'Pro' },
      { id: 'cloud-two', name: '卧室', hardware: 'Mini' },
    ],
  };
  const ui = setup({ initial: data });
  await ui.ready();
  expect(ui.invoke.mock.calls.filter(([command]) => command === 'enable_voice')).toHaveLength(0);
  const source = screen.getByLabelText('听取哪台小爱的指令');
  expect(source).toHaveTextContent('客厅');
  await act(async () => {
    fireEvent.keyDown(source, { key: 'ArrowDown' });
    await vi.advanceTimersByTimeAsync(20);
  });
  fireEvent.click(screen.getByRole('option', { name: /卧室/ }));
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '开始接收指令' }));
  });
  expect(ui.invoke).toHaveBeenCalledWith('enable_voice', {
    sourceId: 'cloud-two',
    targetId: 'speaker',
  });
});

it('Douyin platform parses through the native provider and clearing removes the old preview', async () => {
  const ui = setup({
    panel: 'bilibili',
    operation: async (command) =>
      command === 'parse_video'
        ? {
            id: 'dy-preview',
            provider: 'douyin',
            bvid: '7518247213001592121',
            title: '依然爱你',
            author: '测试作者',
            selected_page: 1,
            parts: [{ cid: 1, page: 1, title: '依然爱你', duration: 330 }],
          }
        : undefined,
  });
  await ui.ready();
  fireEvent.click(screen.getByRole('radio', { name: /抖音/ }));
  expect(document.querySelector('.source-choice')).not.toBeVisible();
  const input = screen.getByLabelText('抖音链接或分享内容');
  fireEvent.change(input, {
    target: { value: 'https://www.douyin.com/video/7518247213001592121' },
  });
  await act(async () => fireEvent.submit(document.querySelector('#bilibili-form')));
  expect(ui.invoke).toHaveBeenCalledWith('parse_video', {
    platform: 'douyin',
    url: 'https://www.douyin.com/video/7518247213001592121',
  });
  expect(screen.getByRole('heading', { name: '依然爱你' })).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: '清除链接', exact: true }));
  expect(input).toHaveValue('');
  expect(input).toHaveFocus();
  expect(screen.queryByRole('heading', { name: '依然爱你' })).toBeNull();
  expect(screen.queryByRole('button', { name: '下载选中音轨' })).toBeNull();
  fireEvent.click(screen.getByRole('radio', { name: /哔哩哔哩/ }));
  expect(screen.getByLabelText('视频链接或 BV 号')).toHaveValue('');
});

it('changing a video link invalidates its preview and a pending parse cannot be cleared or switched', async () => {
  let resolve;
  const ui = setup({
    panel: 'bilibili',
    operation: () =>
      new Promise((done) => {
        resolve = done;
      }),
  });
  await ui.ready();
  const input = screen.getByLabelText('视频链接或 BV 号');
  fireEvent.change(input, { target: { value: 'BV1sVtw6JEeq' } });
  fireEvent.submit(document.querySelector('#bilibili-form'));
  expect(screen.getByRole('button', { name: '清除链接', exact: true })).toBeDisabled();
  expect(screen.getByRole('radio', { name: /抖音/ })).toBeDisabled();
  await act(async () =>
    resolve({
      id: 'bili-preview',
      bvid: 'BV1sVtw6JEeq',
      title: '原视频',
      author: '作者',
      selected_page: 1,
      parts: [{ cid: 10, page: 1, title: '原视频', duration: 60 }],
    }),
  );
  fireEvent.change(input, { target: { value: 'BV1DWLJzaEDv' } });
  expect(screen.queryByRole('heading', { name: '原视频' })).toBeNull();
  expect(screen.queryByRole('button', { name: '下载选中音轨' })).toBeNull();
});
