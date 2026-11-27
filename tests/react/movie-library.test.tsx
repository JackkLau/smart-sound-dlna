import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import MovieLibrary from '../../frontend/src/video/MovieLibrary';
import MagnetDownload from '../../frontend/src/video/MagnetDownload';

const movie = {
  id: '12345',
  title: '测试电影',
  original: 'Test Film',
  year: 1990,
  rating_tenths: 80,
  observed: '2026-11-27',
  kind: 'movie',
  resource_count: 0,
};
const uri = 'magnet:?xt=urn:btih:' + '1'.repeat(40) + '&dn=Test+Film';
const source = 'https://archive.org/details/test-film';
const candidate = { title: 'Test Film 1080p', uri, source, reason: '片名一致，年份一致' };

it('browses offline, requires confirmation before saving a candidate and explicit selection before downloading', async () => {
  const saved: (typeof candidate)[] = [];
  const start = vi.fn().mockResolvedValue(false);
  const invoke = vi.fn(async (command: string) => {
    if (command === 'movie_library_batch_status')
      return { total: 1, processed: 0, pending_review: 0, failed: 0, done: false, message: '' };
    if (command === 'movie_library_candidates') return [];
    if (command === 'movie_library_list') return [movie];
    if (command === 'movie_library_links') return saved.slice();
    if (command === 'movie_library_search') return [candidate];
    if (command === 'movie_library_remove') {
      saved.length = 0;
      return null;
    }
    if (command === 'movie_library_save') {
      saved.push(candidate);
      return null;
    }
    return null;
  });
  render(<MovieLibrary invoke={invoke} onClose={() => {}} start={start} canStart />);
  fireEvent.click(await screen.findByRole('button', { name: /测试电影/ }));
  expect(await screen.findByText('暂无已收录版本。')).toBeVisible();
  expect(invoke).not.toHaveBeenCalledWith('movie_library_search', expect.anything());
  fireEvent.click(screen.getByRole('button', { name: '联网找资源' }));
  expect(await screen.findByText(candidate.reason)).toBeVisible();
  expect(start).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '收录此版本' }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith('movie_library_save', { id: movie.id, uri, source }),
  );
  const versions = await screen.findByRole('list', { name: '已收录版本' });
  fireEvent.click(within(versions).getByRole('button', { name: '选择目录并下载' }));
  await waitFor(() => expect(start).toHaveBeenCalledWith(movie.id, uri));
  expect(await screen.findByText('已取消目录选择。')).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: '移除关联' }));
  expect(invoke).not.toHaveBeenCalledWith('movie_library_remove', expect.anything());
  fireEvent.click(screen.getByRole('button', { name: '确认移除关联' }));
  expect(await screen.findByText('暂无已收录版本。')).toBeVisible();
  expect(invoke).toHaveBeenCalledWith('movie_library_remove', { id: movie.id, uri });
});

it('refreshes local results after a bounded sync and preserves the catalogue when offline', async () => {
  const invoke = vi.fn(async (command: string) => {
    if (command === 'movie_library_batch_status')
      return { total: 1, processed: 0, pending_review: 0, failed: 0, done: false, message: '' };
    if (command === 'movie_library_candidates') return [];
    if (command === 'movie_library_list') return [movie];
    if (command === 'movie_library_sync')
      return { changed: 1, pages: 1, complete: true, message: '本轮更新完成' };
    if (command === 'movie_library_import') throw Error('片单格式有误');
    return [];
  });
  render(<MovieLibrary invoke={invoke} onClose={() => {}} start={async () => false} canStart />);
  await screen.findByRole('button', { name: /测试电影/ });
  fireEvent.click(screen.getByRole('button', { name: '更新豆瓣片库' }));
  expect(await screen.findByText(/本轮更新完成/)).toBeVisible();
  await waitFor(() =>
    expect(
      invoke.mock.calls.filter(([cmd]) => cmd === 'movie_library_list').length,
    ).toBeGreaterThan(1),
  );
  fireEvent.click(screen.getByRole('button', { name: '导入片单' }));
  expect(await screen.findByText('片单格式有误')).toBeVisible();
  expect(screen.getByRole('button', { name: /测试电影/ })).toBeVisible();
});

it('finds a pending review through the filter and pauses a batch after its current request', async () => {
  const second = { ...movie, id: '67890', title: '待核对电影' };
  const initial = {
    processed: 0,
    total: 2,
    pending_review: 1,
    failed: 0,
    done: false,
    message: '',
  };
  let done: (value: typeof initial) => void = () => {};
  const invoke = vi.fn(async (command: string) => {
    if (command === 'movie_library_list') return [movie, second];
    if (command === 'movie_library_review_ids') return [second.id];
    if (command === 'movie_library_batch_status') return initial;
    if (command === 'movie_library_batch_advance')
      return new Promise<typeof initial>((resolve) => {
        done = resolve;
      });
    if (command === 'movie_library_candidates') return [candidate];
    return [];
  });
  render(<MovieLibrary invoke={invoke} onClose={() => {}} start={async () => false} canStart />);
  await screen.findByRole('button', { name: /待核对电影/ });
  fireEvent.change(screen.getByRole('combobox', { name: '筛选资源状态' }), {
    target: { value: 'pending' },
  });
  expect(screen.queryByRole('button', { name: /测试电影/ })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /待核对电影/ }));
  expect(await screen.findByText(candidate.reason)).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: '批量查找资源' }));
  fireEvent.click(await screen.findByRole('button', { name: '暂停查找' }));
  done({ ...initial, processed: 1 });
  await waitFor(() =>
    expect(screen.queryByRole('button', { name: /完成当前影片后暂停/ })).not.toBeInTheDocument(),
  );
  expect(invoke.mock.calls.filter(([cmd]) => cmd === 'movie_library_batch_advance')).toHaveLength(
    1,
  );
});

it('starts a saved version through the native transfer and updates the existing download status', async () => {
  const closeLibrary = vi.fn();
  const invoke = vi.fn(async (command: string) => {
    switch (command) {
      case 'video_magnet_status':
        return null;
      case 'movie_library_list':
        return [{ ...movie, resource_count: 1 }];
      case 'movie_library_links':
        return [{ uri, source }];
      case 'movie_library_start':
        return { id: 'new-task', status: 'fetchingMetadata', files: [], name: 'Test Film' };
      case 'movie_library_batch_status':
        return { total: 1, processed: 0, pending_review: 0, failed: 0, done: false, message: '' };
      default:
        return [];
    }
  });
  render(
    <MagnetDownload
      invoke={invoke}
      libraryOpen
      visible
      ready
      canOpen
      closeLibrary={closeLibrary}
      openFile={async () => {}}
    />,
  );
  fireEvent.click(await screen.findByRole('button', { name: /测试电影/ }));
  const versions = await screen.findByRole('list', { name: '已收录版本' });
  fireEvent.click(within(versions).getByRole('button', { name: '选择目录并下载' }));
  await waitFor(() => expect(closeLibrary).toHaveBeenCalled());
  expect(invoke).toHaveBeenCalledWith('movie_library_start', { id: movie.id, uri });
  expect(await screen.findByText('正在获取影片列表')).toBeInTheDocument();
});

it('refreshes one film while retaining offline evidence, then removes a newly ineligible film with an explanation', async () => {
  let list = [{ ...movie, observed: '2022-08-29' }];
  let attempt = 0;
  const invoke = vi.fn(async (command: string) => {
    if (command === 'movie_library_list') return list.slice();
    if (command === 'movie_library_batch_status')
      return {
        total: list.length,
        processed: 0,
        pending_review: 0,
        failed: 0,
        done: false,
        message: '',
      };
    if (command === 'movie_library_refresh') {
      attempt++;
      if (attempt === 1) throw Error('豆瓣暂时不可用');
      if (attempt === 2) {
        list = [{ ...movie, rating_tenths: 85, observed: '2026-11-26' }];
        return { eligible: true, message: '已更新该片评分。' };
      }
      list = [];
      return { eligible: false, message: '最新评分 7.9，已从当前片库移出。' };
    }
    return [];
  });
  render(<MovieLibrary invoke={invoke} onClose={() => {}} start={async () => false} canStart />);
  fireEvent.click(await screen.findByRole('button', { name: /测试电影/ }));
  const refresh = screen.getByRole('button', { name: '刷新此片评分' });
  fireEvent.click(refresh);
  expect(await screen.findByText('豆瓣暂时不可用')).toBeVisible();
  expect(screen.getByText(/评分采集于 2022-08-29/)).toBeVisible();
  fireEvent.click(refresh);
  expect(await screen.findByText(/评分采集于 2026-11-26/)).toBeVisible();
  expect(screen.getByLabelText('豆瓣评分 8.5')).toBeVisible();
  fireEvent.click(refresh);
  expect(await screen.findByText('最新评分 7.9，已从当前片库移出。')).toBeVisible();
  expect(screen.queryByRole('button', { name: /测试电影/ })).not.toBeInTheDocument();
  expect(screen.queryByRole('region', { name: '影片详情' })).not.toBeInTheDocument();
  expect(invoke).toHaveBeenCalledWith('movie_library_refresh', { id: movie.id });
  expect(invoke).not.toHaveBeenCalledWith('movie_library_sync');
});
