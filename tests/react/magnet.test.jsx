import React from 'react';
import { act, fireEvent, render, screen, cleanup, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import FeaturePanels from '../../frontend/src/features/FeaturePanels.jsx';
import MagnetPanel, { MagnetQueue } from '../../frontend/src/features/MagnetPanel.jsx';

afterEach(cleanup);
const magnet = 'magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=测试文件';

function setup(operation, available = true, read = async () => []) {
  const invoke = vi.fn(async (command, args) => {
    if (command === 'read_magnet_links') return read(args);
    if (command === 'get_features')
      return {
        voice: { devices: [] },
        downloads: { jobs: [] },
        magnets: { available, directory: '/downloads/magnets', jobs: [] },
      };
    return operation?.(command, args);
  });
  const announce = vi.fn();
  render(
    <FeaturePanels
      panel="magnet"
      onClose={vi.fn()}
      invoke={invoke}
      connected
      refresh={vi.fn()}
      announce={announce}
    />,
  );
  return { invoke, announce };
}

it('validates a magnet before invoking native download and focuses its field', async () => {
  const { invoke } = setup();
  const field = await screen.findByLabelText('磁力链接');
  fireEvent.change(field, { target: { value: 'https://example.com/file' } });
  fireEvent.submit(field.closest('form'));
  await screen.findByText('请粘贴包含有效 BTIH 的 magnet:? 链接。');
  expect(invoke.mock.calls.some(([command]) => command === 'add_magnet')).toBe(false);
  expect(field).toHaveFocus();
});

it('prevents duplicate submission and preserves a failed draft for retry', async () => {
  let reject;
  const { invoke } = setup(
    () =>
      new Promise((_, fail) => {
        reject = fail;
      }),
  );
  const field = await screen.findByLabelText('磁力链接');
  fireEvent.change(field, { target: { value: magnet } });
  fireEvent.submit(field.closest('form'));
  fireEvent.submit(field.closest('form'));
  expect(invoke.mock.calls.filter(([command]) => command === 'add_magnet')).toHaveLength(1);
  expect(screen.getByRole('button', { name: '开始下载' })).toBeDisabled();
  await act(async () => {
    reject(new Error('磁盘空间不足'));
  });
  expect(await screen.findByText('磁盘空间不足')).toBeVisible();
  expect(field).toHaveValue(magnet);
  expect(field).toHaveFocus();
});

it('submits a real IPC command, clears only on success and announces it', async () => {
  const { invoke, announce } = setup(async () => ({ id: 'hash' }));
  const field = await screen.findByLabelText('磁力链接');
  fireEvent.change(field, { target: { value: magnet } });
  fireEvent.submit(field.closest('form'));
  await waitFor(() => expect(field).toHaveValue(''));
  expect(invoke).toHaveBeenCalledWith('add_magnet', { uri: magnet });
  expect(announce).toHaveBeenCalledWith('已添加磁力下载任务');
});

it('explains the missing engine and leaves the draft editable', async () => {
  setup(undefined, false);
  expect(await screen.findByText(/尚未找到 aria2/)).toBeVisible();
  expect(screen.getByLabelText('磁力链接')).toBeEnabled();
  expect(screen.getByRole('button', { name: '开始下载' })).toBeDisabled();
});

function queueController(jobs) {
  const invoke = vi.fn(async () => {});
  return {
    features: { magnets: { available: true, jobs } },
    disabled: false,
    pending: '',
    errors: {},
    invoke,
    announce: vi.fn(),
    run: async (_key, operation) => operation(),
  };
}

it('keeps metadata progress indeterminate and dispatches explicit queue controls', () => {
  const controller = queueController([
    { id: 'resource', title: '资源', phase: 'resolving', downloaded: 0, total: 16000 },
  ]);
  const view = render(<MagnetQueue controller={controller} />);
  expect(screen.getByRole('progressbar')).not.toHaveAttribute('value');
  fireEvent.click(screen.getByRole('button', { name: '暂停 资源' }));
  expect(controller.invoke).toHaveBeenCalledWith('control_magnet', {
    id: 'resource',
    action: 'pause',
  });
  view.rerender(
    <MagnetQueue
      controller={{
        ...controller,
        features: {
          magnets: {
            available: true,
            jobs: [{ ...controller.features.magnets.jobs[0], phase: 'failed' }],
          },
        },
      }}
    />,
  );
  fireEvent.click(screen.getByRole('button', { name: '停止 资源' }));
  expect(controller.invoke).toHaveBeenCalledWith('control_magnet', {
    id: 'resource',
    action: 'stop',
  });
});

it('clamps pagination after clearing and confirms history removal with Cancel focused', async () => {
  const controller = queueController(
    Array.from({ length: 11 }, (_, index) => ({
      id: String(index),
      title: `资源 ${index}`,
      phase: 'completed',
      downloaded: 1024,
      total: 1024,
    })),
  );
  const view = render(<MagnetQueue controller={controller} />);
  expect(screen.getAllByRole('listitem')).toHaveLength(10);
  fireEvent.click(screen.getByRole('button', { name: '下一页' }));
  expect(screen.getAllByRole('listitem')).toHaveLength(1);
  fireEvent.click(screen.getByRole('button', { name: '清除结束记录（保留文件）' }));
  expect(screen.getByRole('button', { name: '取消', exact: true })).toHaveFocus();
  expect(controller.invoke).not.toHaveBeenCalledWith('clear_finished_magnets');
  fireEvent.click(screen.getByRole('button', { name: '清除记录', exact: true }));
  await waitFor(() => expect(controller.invoke).toHaveBeenCalledWith('clear_finished_magnets'));
  const next = {
    ...controller,
    features: { magnets: { available: true, jobs: controller.features.magnets.jobs.slice(0, 2) } },
  };
  view.rerender(<MagnetQueue controller={next} />);
  expect(screen.getAllByRole('listitem')).toHaveLength(2);
  expect(screen.getByRole('button', { name: '下一页' })).toBeDisabled();
});

const candidate = {
  id: '0123456789abcdef0123456789abcdef01234567',
  title: '测试文件',
  uri: magnet,
};
const other = {
  id: 'f'.repeat(40),
  title: '第二个文件',
  uri: 'magnet:?xt=urn:btih:' + 'f'.repeat(40),
};

it('fills a single clipboard magnet on entry and waits for explicit download', async () => {
  const { invoke } = setup(undefined, true, async () => [candidate]);
  expect(await screen.findByDisplayValue(magnet)).toBeVisible();
  expect(invoke).not.toHaveBeenCalledWith('add_magnet', expect.anything());
});

it('reads again on focus and keeps a manually edited draft until a candidate is chosen', async () => {
  const read = vi.fn().mockResolvedValueOnce([candidate]).mockResolvedValue([candidate, other]);
  const { invoke } = setup(undefined, true, read);
  const field = await screen.findByDisplayValue(magnet);
  fireEvent.change(field, { target: { value: '正在编辑' } });
  fireEvent.focus(window);
  const choice = await screen.findByRole('button', { name: /第二个文件/ });
  expect(field).toHaveValue('正在编辑');
  fireEvent.click(choice);
  expect(field).toHaveValue(other.uri);
  expect(invoke).not.toHaveBeenCalledWith('add_magnet', expect.anything());
});

it('discards a late clipboard read after typing or closing', async () => {
  let finish;
  const read = vi.fn(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const ctrl = {
    features: { magnets: { available: true } },
    invoke: read,
    clearError: vi.fn(),
    disabled: false,
    pending: '',
    errors: {},
    run: vi.fn(),
  };
  const props = { controller: ctrl, onClose: vi.fn() };
  const view = render(<MagnetPanel open {...props} />);
  const field = screen.getByLabelText('磁力链接');
  fireEvent.change(field, { target: { value: '我的草稿' } });
  await act(async () => finish([candidate]));
  expect(field).toHaveValue('我的草稿');
  fireEvent.focus(window);
  view.rerender(<MagnetPanel open={false} {...props} />);
  await act(async () => finish([candidate]));
  fireEvent.focus(window);
  expect(read).toHaveBeenCalledTimes(2);
  expect(field).toHaveValue('我的草稿');
});

it('uses the same parser on explicit paste when clipboard reading is denied', async () => {
  const read = vi.fn().mockRejectedValueOnce(new Error('denied')).mockResolvedValue([candidate]);
  setup(undefined, true, read);
  expect(await screen.findByText(/无法读取剪贴板/)).toBeVisible();
  const field = screen.getByLabelText('磁力链接');
  const text = '分享给你：' + magnet;
  fireEvent.paste(field, { clipboardData: { getData: () => text } });
  expect(await screen.findByDisplayValue(magnet)).toBeVisible();
  expect(read).toHaveBeenLastCalledWith({ text });
});
