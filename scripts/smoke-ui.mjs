import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { existsSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { chromium, expect } from '@playwright/test';

const roots = { desktop: 'dist/desktop', mobile: 'dist/mobile', legacy: 'xiaomi_dlna/static/dist' };
const csp =
  "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";
const server = createServer(async (request, response) => {
  if (request.url === '/favicon.ico') {
    response.writeHead(204).end();
    return;
  }
  const [, surface, asset = 'index.html'] = new URL(request.url, 'http://localhost').pathname.split(
    '/',
  );
  if (!roots[surface] || !['index.html', 'app.js', 'app.css', ''].includes(asset)) {
    response.writeHead(404).end();
    return;
  }
  try {
    const file = asset || 'index.html';
    let body = await readFile(resolve(roots[surface], file));
    let policy = csp;
    if (surface === 'desktop' && file === 'index.html') {
      // Mirror Tauri's per-response nonce injection for bundled style elements.
      const nonce = randomBytes(24).toString('base64');
      const html = body.toString();
      if (!html.includes('<style id="ui-style-nonce">'))
        throw new Error('Desktop HTML is missing the CSP style nonce carrier');
      body = html.replace(
        '<style id="ui-style-nonce">',
        `<style id="ui-style-nonce" nonce="${nonce}">`,
      );
      policy = csp.replace("style-src 'self'", `style-src 'self' 'nonce-${nonce}'`);
    }
    response.writeHead(200, {
      'Content-Type': `${file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html'}; charset=utf-8`,
      'Content-Security-Policy': policy,
    });
    response.end(body);
  } catch (error) {
    response.writeHead(500).end(error.message);
  }
});
await new Promise((ready) => server.listen(0, '127.0.0.1', ready));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({
  executablePath:
    process.env.CHROMIUM_PATH ||
    (existsSync('/usr/bin/chromium') ? '/usr/bin/chromium' : undefined),
  headless: true,
});
await mkdir('test-results/react', { recursive: true });
const errors = [];
const watch = (page) => {
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (entry) => {
    if (entry.type() === 'error' && !entry.text().includes('favicon.ico'))
      errors.push(entry.text());
  });
};
try {
  const page = await browser.newPage({
    viewport: { width: 1180, height: 850 },
    reducedMotion: 'reduce',
  });
  watch(page);
  await page.addInitScript(() => {
    const tracks = [
      { id: 'a', title: '晴天', filename: '晴天.mp3', size: 2048, format: 'mp3' },
      { id: 'b', title: '海阔天空', filename: '海阔天空.flac', size: 4096, format: 'flac' },
    ];
    const state = {
      tracks,
      devices: [
        {
          id: 'speaker',
          name: '小爱音箱 Pro',
          model: 'Pro',
          address: '192.168.1.5',
          is_xiaomi: true,
          is_xiaomi_pro: true,
        },
      ],
      selected_device_id: 'speaker',
      device_connection: 'connected',
      current_track_id: null,
      transport_state: 'STOPPED',
      volume: 30,
      duration: 180,
      position: 0,
      queue_mode: 'sequential',
      warnings: [],
    };
    window.fixtureCalls = [];
    window.__TAURI__ = {
      core: {
        invoke: async (command, args) => {
          window.fixtureCalls.push({ command, args });
          if (command === 'get_config') return { formats: ['mp3', 'flac'], max_tracks: 20000 };
          if (command === 'get_features')
            return {
              voice: { devices: [], connected: false, listening: false },
              downloads: { jobs: [], directory: '', authenticated: false },
            };
          if (command === 'recognize_track_name')
            return { title: '晴天现场版', source: 'audio_tag' };
          if (command === 'rename_track')
            state.tracks = state.tracks.map((track) =>
              track.id === args.trackId ? { ...track, title: args.title } : track,
            );
          if (command === 'remove_track')
            state.tracks = state.tracks.filter((track) => track.id !== args.trackId);
          if (command === 'play_track') {
            state.current_track_id = args.trackId;
            state.transport_state = 'PLAYING';
          }
          if (command === 'control') {
            if (args.action === 'pause') state.transport_state = 'PAUSED_PLAYBACK';
            if (args.action === 'mode') state.queue_mode = args.value;
          }
          if (command === 'pick_music_files') return null;
          return structuredClone(state);
        },
      },
    };
  });
  await page.goto(`${origin}/desktop/`);
  await expect(page.locator('#connection-status')).toHaveText('客户端已就绪');
  await page.getByRole('button', { name: '播放 晴天', exact: true }).click();
  await expect(page.locator('#now-title')).toHaveText('晴天');
  await page.getByRole('button', { name: '暂停', exact: true }).click();
  await expect(page.locator('#now-subtitle')).toContainText('已暂停');
  await page.getByRole('button', { name: '识别或修改曲名：晴天', exact: true }).click();
  await expect(page.getByLabel('曲名', { exact: true })).toBeFocused();
  await page.getByRole('button', { name: '识别曲名', exact: true }).click();
  await expect(page.getByLabel('曲名', { exact: true })).toHaveValue('晴天现场版');
  await page.getByRole('button', { name: '保存曲名', exact: true }).click();
  await expect(page.locator('#track-name-dialog')).not.toBeVisible();
  await expect(
    page.getByRole('button', { name: '识别或修改曲名：晴天现场版', exact: true }),
  ).toBeFocused();
  await page.getByLabel('搜索音乐', { exact: true }).fill('不存在');
  await expect(page.locator('#empty-title')).toHaveText('没有找到这首歌');
  await page.getByRole('button', { name: '清空搜索', exact: true }).first().click();
  await expect(page.getByLabel('搜索音乐', { exact: true })).toBeFocused();
  await page.getByRole('combobox', { name: '播放顺序', exact: true }).click();
  await expect(page.getByRole('option', { name: '列表循环', exact: true })).toBeVisible();
  expect(await page.evaluate(() => getComputedStyle(document.body).overflow)).toBe('hidden');
  expect(
    await page.evaluate(() => {
      const nonce = document.getElementById('ui-style-nonce').nonce;
      const styles = [...document.querySelectorAll('style')];
      return Boolean(nonce) && styles.length >= 3 && styles.every((style) => style.nonce === nonce);
    }),
  ).toBe(true);
  await page.getByRole('option', { name: '列表循环', exact: true }).click();
  await expect(page.getByRole('combobox', { name: '播放顺序', exact: true })).toHaveText(
    '列表循环',
  );
  await page.screenshot({ path: 'test-results/react/desktop.png', fullPage: true });
  await page.getByRole('button', { name: '通过设备地址添加' }).click();
  await page.getByRole('button', { name: '添加音响', exact: true }).click();
  await expect(page.locator('#device-error')).toContainText('请输入完整');
  await page.keyboard.press('Escape');
  await expect(page.locator('#device-dialog')).not.toBeVisible();
  await expect(page.getByRole('button', { name: '通过设备地址添加' })).toBeFocused();
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(() => document.body.scrollWidth <= document.documentElement.clientWidth),
  ).toBe(true);
  await page.screenshot({ path: 'test-results/react/desktop-narrow.png', fullPage: true });
  await page.close();

  const phone = await browser.newPage({
    viewport: { width: 390, height: 844 },
    reducedMotion: 'reduce',
  });
  watch(phone);
  const phoneCalls = [];
  let player = {
    connection: 'connected',
    device_id: 'speaker',
    device_name: '小爱音箱 Pro',
    current_track_id: null,
    transport: 'STOPPED',
    mode: 'sequential',
    title: '',
  };
  await phone.addInitScript(() => localStorage.setItem('mobile-session-v1', 'fixture-phone-token'));
  await phone.route('**/api/**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const body = request.postDataJSON();
    phoneCalls.push({ path: url.pathname, body });
    let data;
    if (url.pathname === '/api/jobs') data = { jobs: [] };
    else if (url.pathname === '/api/library')
      data = {
        player,
        tracks: [{ id: 'a', title: '晴天', format: 'mp3' }],
        total: 1,
        library_total: 1,
        page: 1,
        pages: 1,
      };
    else if (url.pathname === '/api/playback') {
      player = {
        ...player,
        ...(body.action === 'mode'
          ? { mode: body.mode }
          : { current_track_id: 'a', transport: 'PLAYING', title: '晴天' }),
      };
      data = player;
    } else if (url.pathname === '/api/parse')
      data = {
        id: 'preview',
        bvid: 'BVfixture',
        title: '本地测试音轨',
        author: '测试作者',
        parts: [{ cid: 1, page: 1, title: '现场版' }],
        selected_page: 1,
      };
    else if (url.pathname === '/api/download')
      data = {
        items: [{ cid: 1, accepted: true, job: { id: 'download' } }],
        queued: 1,
        skipped: 0,
        failed: 0,
      };
    else throw new Error(`Unexpected phone request: ${url.pathname}`);
    await route.fulfill({ json: data });
  });
  await phone.goto(`${origin}/mobile/`);
  await phone.getByRole('button', { name: '播放 晴天', exact: true }).click();
  await expect(phone.locator('#playing-title')).toHaveText('晴天');
  await phone.getByLabel('播放模式', { exact: true }).selectOption('repeat-one');
  await phone.getByRole('button', { name: '应用播放模式' }).click();
  await expect(phone.locator('#phone-queue-help')).toContainText('单曲循环');
  await phone.getByLabel('搜索曲名', { exact: true }).fill('晴天');
  await phone.getByRole('button', { name: '清空搜索曲名' }).click();
  await expect(phone.getByLabel('搜索曲名', { exact: true })).toHaveValue('');
  await expect(phone.getByLabel('搜索曲名', { exact: true })).toBeFocused();
  await phone.getByLabel('视频链接或分享文案', { exact: true }).fill('旧的分享文案');
  await phone.getByRole('button', { name: '清空视频链接或分享文案' }).click();
  await expect(phone.getByLabel('视频链接或分享文案', { exact: true })).toHaveValue('');
  await expect(phone.getByLabel('视频链接或分享文案', { exact: true })).toBeFocused();
  await expect(phone.getByRole('button', { name: '清空视频链接或分享文案' })).not.toBeVisible();
  await phone.getByLabel('视频链接或分享文案', { exact: true }).fill('BVfixture');
  await phone.getByRole('button', { name: '发送并解析' }).click();
  await expect(phone.locator('#video-title')).toHaveText('本地测试音轨');
  await phone.getByRole('button', { name: '在电脑下载选中音轨' }).click();
  await expect(phone.locator('#download-summary')).toContainText('1');
  expect(phoneCalls.filter((call) => call.path === '/api/download')).toHaveLength(1);
  await phone.screenshot({ path: 'test-results/react/mobile.png', fullPage: true });
  await phone.getByRole('button', { name: '清空视频链接或分享文案' }).click();
  await expect(phone.getByLabel('视频链接或分享文案', { exact: true })).toHaveValue('');
  await expect(phone.getByLabel('视频链接或分享文案', { exact: true })).toBeFocused();
  await expect(phone.locator('#download-form')).not.toBeVisible();

  await phone.close();

  const legacy = await browser.newPage({ viewport: { width: 1180, height: 850 } });
  watch(legacy);
  await legacy.route('**/api/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    const data =
      path === '/api/config'
        ? { control_token: 'fixture-control', formats: ['mp3'], max_upload_bytes: 1024 * 1024 }
        : {
            tracks: [],
            devices: [],
            selected_device_id: null,
            current_track_id: null,
            transport_state: 'STOPPED',
            volume: null,
            queue_mode: 'sequential',
          };
    await route.fulfill({ json: data });
  });
  await legacy.goto(`${origin}/legacy/`);
  await expect(legacy.locator('#connection-status')).toContainText('本地服务已连接');
  await expect(legacy.getByRole('heading', { name: '这里，留给你的好音乐' })).toBeVisible();
  await legacy.screenshot({ path: 'test-results/react/legacy.png', fullPage: true });
  await legacy.close();
  expect(errors).toEqual([]);
  console.log(
    'PASS: desktop IPC flows, native dialog focus/Escape, search, keyboard modes, narrow layout, mobile pairing/playback/import/input clearing, legacy rendering, production CSP; no browser errors.',
  );
} finally {
  await browser.close();
  await new Promise((done) => server.close(done));
}
