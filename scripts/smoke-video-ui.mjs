import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { existsSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { chromium, expect } from '@playwright/test';

const server = createServer(async (request, response) => {
  const asset = new URL(request.url, 'http://localhost').pathname.slice(1) || 'index.html';
  if (asset === 'favicon.ico') return response.writeHead(204).end();
  if (!['index.html', 'app.js', 'app.css'].includes(asset)) return response.writeHead(404).end();
  try {
    let body = await readFile(resolve('dist/desktop', asset));
    const nonce = randomBytes(24).toString('base64');
    if (asset === 'index.html') {
      body = Buffer.from(
        body
          .toString()
          .replace('<style id="ui-style-nonce">', `<style id="ui-style-nonce" nonce="${nonce}">`),
      );
    }
    response
      .writeHead(200, {
        'Content-Type': asset.endsWith('.js')
          ? 'text/javascript'
          : asset.endsWith('.css')
            ? 'text/css'
            : 'text/html',
        'Content-Security-Policy': `default-src 'self'; script-src 'self'; style-src 'self' 'nonce-${nonce}'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'`,
      })
      .end(body);
  } catch (error) {
    response.writeHead(500).end(error.message);
  }
});
await new Promise((ready) => server.listen(0, '127.0.0.1', ready));
const browser = await chromium.launch({
  executablePath:
    process.env.CHROMIUM_PATH ||
    (existsSync('/usr/bin/chromium') ? '/usr/bin/chromium' : undefined),
  headless: true,
});
await mkdir('test-results/react', { recursive: true });
try {
  const page = await browser.newPage({
    viewport: { width: 1180, height: 850 },
    reducedMotion: 'reduce',
  });
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (event) => {
    if (event.type() === 'error') errors.push(event.text());
  });
  await page.addInitScript(() => {
    const music = {
      tracks: [{ id: 'song', title: '晴天', filename: '晴天.mp3', size: 2048, format: 'mp3' }],
      devices: [
        {
          id: 'speaker',
          name: '小爱音箱 Pro',
          model: 'Pro',
          address: '192.168.1.5',
          is_xiaomi: true,
          is_xiaomi_pro: true,
        },
        {
          id: 'tv',
          name: '创维 A7H',
          model: 'A7H',
          address: '192.168.1.6',
          is_xiaomi: false,
          is_xiaomi_pro: false,
        },
      ],
      selected_device_id: 'speaker',
      device_connection: 'connected',
      current_track_id: 'song',
      transport_state: 'PLAYING',
      volume: 30,
      duration: 180,
      position: 30,
      queue_mode: 'repeat-all',
      warnings: [],
      playback_context: {
        active_kind: 'audio',
        owner_revision: 1,
        music_control_blocked_reason: null,
      },
    };
    const video = {
      enabled: true,
      recent_videos: [],
      catalog_warning: null,
      entry: null,
      probe: null,
      plan: null,
      session: null,
      owner_kind: 'audio',
      owner_revision: 1,
      music_active: true,
      error: null,
    };
    window.videoCalls = [];
    window.failVideoPause = false;
    window.failVideoRecheck = false;
    window.cancelVideoExport = true;
    window.__TAURI__ = {
      core: {
        invoke: async (command, args = {}) => {
          window.videoCalls.push({ command, args });
          if (command === 'get_config')
            return { formats: ['mp3', 'flac'], max_tracks: 20000, video_casting_enabled: true };
          if (command === 'get_features')
            return {
              voice: { devices: [], connected: false, listening: false },
              downloads: { jobs: [], directory: '', authenticated: false },
            };
          if (command === 'get_state') return structuredClone(music);
          if (command === 'get_video_state') return structuredClone(video);
          if (command === 'pick_video_file') {
            video.entry = {
              id: 'film',
              title: '群山与海洋 4K HDR10 原文件演示.mkv',
              format: 'mkv',
              size: '85899345920',
              duration_ms: 7200000,
              video_codec: 'hevc',
              width: 3840,
              height: 2160,
              pixel_format: 'yuv420p10le',
              color_transfer: 'smpte2084',
              audio_tracks: [
                { index: 1, codec: 'eac3', language: 'zho', title: '国语', channels: 6 },
              ],
              subtitle_tracks: [
                { index: 2, codec: 'subrip', language: 'zho', title: '中文字幕', channels: null },
              ],
            };
            video.probe = { task_id: 'probe', status: 'ready', error: null };
            video.recent_videos = Array.from({ length: 7 }, (_, index) => ({
              id: index === 0 ? 'film' : `recent-${index}`,
              title:
                index === 0
                  ? video.entry.title
                  : `另一部很长的影片名称 第 ${index} 部 电影完整版.mkv`,
              format: 'mkv',
              size: '85899345920',
              duration_ms: 7200000,
              video_codec: 'hevc',
              last_selected_ms: 1000,
              progress: { position_ms: 45000, duration_ms: 7200000, observed_at_ms: 1000 },
            }));
          } else if (command === 'select_recent_video') {
            if (window.failRecentSelection)
              throw new Error('SOURCE_CHANGED：影片文件已改变，请重新选择原文件。');
            const entry = video.entry;
            video.entry = null;
            video.plan = null;
            video.probe = { task_id: 'recent-probe', status: 'running', error: null };
            window.completeRecentProbe = () => {
              video.entry = entry;
              video.probe.status = 'ready';
            };
          } else if (command === 'forget_recent_video') {
            if (window.failRecentRemoval) throw new Error('最近影片记录未保存，请重试。');
            video.recent_videos = video.recent_videos.filter((entry) => entry.id !== args.mediaId);
          } else if (command === 'prepare_video_play') {
            video.plan = {
              id: 'plan',
              media_id: 'film',
              device_id: args.deviceId,
              mode: 'trial',
              reason: '尚未完成真机格式验证，可试播原文件。',
              can_pause: true,
              can_seek: true,
            };
          } else if (command === 'start_video_play') {
            if (video.music_active && !args.replaceCurrent) throw new Error('请明确切换当前音乐。');
            video.owner_revision += 1;
            video.owner_kind = 'video';
            video.music_active = false;
            video.session = {
              id: 'session',
              generation: video.owner_revision,
              media_id: 'film',
              device_id: 'tv',
              title: video.entry.title,
              phase: 'playing',
              position_ms: 180000,
              duration_ms: 7200000,
              can_pause: true,
              can_seek: true,
              command_accepted: true,
              http_requested: true,
              transport_state: 'PLAYING',
              error: null,
              diagnostics: {
                schema_version: 1,
                elapsed_ms: 180000,
                last_command: { action: 'Play', outcome: 'accepted', elapsed_ms: 45 },
                last_poll_age_ms: 1000,
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
            };
            video.plan = null;
            music.current_track_id = null;
            music.position = 0;
            music.duration = 0;
            music.transport_state = 'STOPPED';
            music.playback_context = {
              active_kind: 'video',
              owner_revision: video.owner_revision,
              music_control_blocked_reason: '当前正在影视播放，请先在电脑切换到音乐。',
            };
          } else if (command === 'export_video_diagnostics') {
            if (window.cancelVideoExport) {
              window.cancelVideoExport = false;
              return false;
            }
            return true;
          } else if (command === 'recheck_video_session') {
            if (window.failVideoRecheck) {
              video.session.diagnostics.consecutive_poll_failures += 1;
              throw new Error('电视仍未响应，请检查网络后重试。');
            }
            video.session.phase = 'playing';
            video.session.error = null;
            video.session.diagnostics.consecutive_poll_failures = 0;
            video.owner_kind = 'video';
          } else if (command === 'control_video') {
            if (args.action === 'pause' && window.failVideoPause) {
              video.session.phase = 'remote_state_unknown';
              video.owner_kind = 'unknown';
              video.session.error = '电视暂未确认暂停，请核对电视状态。';
              throw new Error(video.session.error);
            }
            if (args.action === 'stop') {
              video.session.phase = 'stopped';
              video.session.error = null;
              video.owner_kind = 'none';
              video.owner_revision += 1;
              music.playback_context = {
                active_kind: 'none',
                owner_revision: video.owner_revision,
                music_control_blocked_reason: null,
              };
            }
            if (args.action === 'pause') video.session.phase = 'paused';
            if (args.action === 'resume') video.session.phase = 'playing';
          } else if (command === 'start_music_play') {
            video.owner_kind = 'audio';
            video.owner_revision += 1;
            video.music_active = true;
            video.session.phase = 'stopped';
            video.session.error = null;
            music.current_track_id = args.trackId;
            music.transport_state = 'PLAYING';
            music.duration = 180;
            music.playback_context = {
              active_kind: 'audio',
              owner_revision: video.owner_revision,
              music_control_blocked_reason: null,
            };
            return structuredClone(music);
          } else {
            return structuredClone(music);
          }
          return structuredClone(video);
        },
      },
    };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  const navigation = page.getByRole('navigation', { name: '媒体类型' });
  await navigation.getByRole('button', { name: '影视', exact: true }).click();
  await expect(page.locator('#video-title')).toBeVisible();
  await page.getByRole('button', { name: '选择影片', exact: true }).click();
  const recent = page.getByRole('region', { name: '最近影片', exact: true });
  await expect(recent.getByRole('listitem')).toHaveCount(5);
  await recent.getByRole('button', { name: '显示更多影片', exact: true }).click();
  await expect(recent.getByRole('listitem')).toHaveCount(7);
  const recentSelect = recent.getByRole('button', {
    name: '重新选择 群山与海洋 4K HDR10 原文件演示.mkv',
    exact: true,
  });
  await page.evaluate(() => {
    window.failRecentSelection = true;
  });
  await recentSelect.click();
  await expect(page.getByRole('alert').filter({ hasText: '影片文件已改变' }).first()).toBeVisible();
  await page.evaluate(() => {
    window.failRecentSelection = false;
  });
  await recentSelect.click();
  await expect(
    page.getByText('正在读取影片信息，不会复制或转换原文件。', { exact: true }),
  ).toBeVisible();
  await expect(recentSelect).toBeDisabled();
  await expect(page.getByRole('heading', { name: '影片文件', exact: true })).toBeFocused();
  await page.evaluate(() => window.completeRecentProbe());
  await expect(recentSelect).toBeEnabled();
  await recent.scrollIntoViewIfNeeded();
  await page.screenshot({ path: 'test-results/react/video-recent.png' });

  await page.getByRole('combobox', { name: '候选电视', exact: true }).click();
  await expect(page.getByRole('option', { name: /创维 A7H/ })).toBeVisible();
  await page.screenshot({ path: 'test-results/react/video-device-select.png' });
  await expect(page.getByRole('option', { name: /创维 A7H/ })).toBeVisible();
  await page.getByRole('option', { name: /创维 A7H/ }).click();
  await page.getByRole('button', { name: '检查并准备', exact: true }).click();
  expect(
    await page.evaluate(() =>
      window.videoCalls.some(({ command }) =>
        ['control', 'select_device', 'start_video_play', 'play_track'].includes(command),
      ),
    ),
  ).toBe(false);
  await page.getByRole('button', { name: '切换并播放影片', exact: true }).click();
  await expect(page.getByRole('dialog', { name: '切换到影片', exact: true })).toBeVisible();
  expect(
    await page.evaluate(() =>
      window.videoCalls.some(({ command }) => command === 'start_video_play'),
    ),
  ).toBe(false);
  await page.getByRole('button', { name: '停止当前内容并播放', exact: true }).click();
  await expect(page.getByRole('button', { name: '暂停影片', exact: true })).toBeVisible();
  await page.screenshot({ path: 'test-results/react/video-playing.png', fullPage: true });
  const diagnosticToggle = page.getByText('连接与传输诊断', { exact: true });
  await diagnosticToggle.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByText('8,589,934,592 字节', { exact: true })).toBeVisible();
  await expect(page.getByText('GET 3 · HEAD 1 · Range 2', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: '导出诊断', exact: true }).click();
  await expect(page.getByText('诊断已保存。', { exact: true })).not.toBeVisible();
  await page.getByRole('button', { name: '导出诊断', exact: true }).click();
  await expect(page.getByText('诊断已保存。', { exact: true })).toBeVisible();
  await diagnosticToggle.scrollIntoViewIfNeeded();
  await page.screenshot({ path: 'test-results/react/video-diagnostics.png' });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(() => document.body.scrollWidth <= document.documentElement.clientWidth),
  ).toBe(true);
  await expect(page.locator('header.topbar')).toHaveCount(1);
  await page.evaluate(
    () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
  );
  await page.screenshot({ path: 'test-results/react/video-narrow.png' });
  await page.getByRole('combobox', { name: '候选电视', exact: true }).scrollIntoViewIfNeeded();
  await expect(page.getByRole('combobox', { name: '候选电视', exact: true })).toBeVisible();
  await page.screenshot({ path: 'test-results/react/video-narrow-device.png' });
  await diagnosticToggle.scrollIntoViewIfNeeded();
  expect(
    await page.evaluate(() => document.body.scrollWidth <= document.documentElement.clientWidth),
  ).toBe(true);
  await page.screenshot({ path: 'test-results/react/video-diagnostics-narrow.png' });
  await recent.getByRole('heading', { name: '最近影片', exact: true }).scrollIntoViewIfNeeded();
  expect(
    await page.evaluate(() => document.body.scrollWidth <= document.documentElement.clientWidth),
  ).toBe(true);
  await page.screenshot({ path: 'test-results/react/video-recent-narrow.png' });
  const removeRecent = recent.getByRole('button', {
    name: '移除记录 群山与海洋 4K HDR10 原文件演示.mkv',
    exact: true,
  });
  await removeRecent.click();
  const forgetDialog = page.getByRole('dialog', { name: '移除影片记录', exact: true });
  await expect(forgetDialog.getByRole('button', { name: '取消', exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(removeRecent).toBeFocused();
  await removeRecent.click();
  await page.evaluate(() => {
    window.failRecentRemoval = true;
  });
  await forgetDialog.getByRole('button', { name: '移除记录', exact: true }).click();
  await expect(forgetDialog.getByRole('alert')).toContainText('最近影片记录未保存');
  await expect(removeRecent).toHaveCount(1);
  await page.screenshot({ path: 'test-results/react/video-recent-removal-error.png' });
  await page.evaluate(() => {
    window.failRecentRemoval = false;
  });
  await forgetDialog.getByRole('button', { name: '移除记录', exact: true }).click();
  await expect(forgetDialog).not.toBeVisible();
  await expect(recent.getByRole('heading', { name: '最近影片', exact: true })).toBeFocused();
  await expect(removeRecent).toHaveCount(0);
  await expect(page.getByRole('button', { name: '暂停影片', exact: true })).toBeEnabled();
  expect(
    await page.evaluate(() => window.videoCalls.some(({ command }) => command === 'control_video')),
  ).toBe(false);

  await page.evaluate(() => {
    window.failVideoPause = true;
  });
  await page.getByRole('button', { name: '暂停影片', exact: true }).click();
  await expect(
    page.getByRole('alert').filter({ hasText: '电视暂未确认暂停' }).first(),
  ).toBeVisible();
  await page.evaluate(() => {
    window.failVideoRecheck = true;
  });
  await page.getByRole('button', { name: '重新检查连接', exact: true }).click();
  await expect(page.getByRole('alert').filter({ hasText: '电视仍未响应' }).first()).toBeVisible();
  await page.evaluate(() => {
    window.failVideoRecheck = false;
  });
  await page.getByRole('button', { name: '重新检查连接', exact: true }).click();
  await expect(page.getByRole('button', { name: '暂停影片', exact: true })).toBeEnabled();
  expect(
    await page.evaluate(
      () => window.videoCalls.filter(({ command }) => command === 'start_video_play').length,
    ),
  ).toBe(1);
  await page.getByRole('button', { name: '停止影片', exact: true }).click();
  await expect(page.getByRole('button', { name: '停止影片', exact: true })).not.toBeVisible();
  await page.setViewportSize({ width: 1180, height: 850 });
  await page.getByRole('button', { name: '检查并准备', exact: true }).click();
  await page.getByRole('button', { name: '试播原文件', exact: true }).click();
  await expect(page.getByRole('button', { name: '暂停影片', exact: true })).toBeVisible();
  await navigation.getByRole('button', { name: '音乐', exact: true }).click();
  await page.getByRole('button', { name: '切换到音乐并播放 晴天', exact: true }).click();
  await expect(page.getByRole('dialog', { name: '切换回音乐', exact: true })).toBeVisible();
  await page.getByRole('button', { name: /停止.*并播放音乐/ }).click();
  await expect(page.locator('#now-title')).toHaveText('晴天');
  expect(
    await page.evaluate(
      () => window.videoCalls.filter(({ command }) => command === 'start_music_play').length,
    ),
  ).toBe(1);
  expect(errors).toEqual([]);
  console.log(
    'Video browser flow passed: recent history/selection/removal, preparation isolation, explicit handoff, diagnostics/export cancellation, explicit recheck without replay, failure, recovery, select popup and narrow layout.',
  );
} finally {
  await browser.close();
  await new Promise((done) => server.close(done));
}
