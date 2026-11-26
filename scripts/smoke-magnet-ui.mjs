import { chromium, expect } from '@playwright/test';
import { readFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';

const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM_PATH || (existsSync(chrome) ? chrome : undefined),
  headless: true,
});
const errors = [];
try {
  const page = await browser.newPage({
    viewport: { width: 1180, height: 850 },
    reducedMotion: 'reduce',
  });
  page.on('pageerror', (error) => errors.push(error.message));
  // Fulfil built resources directly: this test needs no HTTP listener or external network.
  await page.route('http://magnet.test/**', async (route) => {
    const asset = new URL(route.request().url()).pathname.slice(1) || 'index.html';
    if (!['index.html', 'app.js', 'app.css'].includes(asset)) return route.fulfill({ status: 404 });
    let body = await readFile(resolve('dist/desktop', asset));
    if (asset === 'index.html')
      body = Buffer.from(
        body
          .toString()
          .replace(
            '<style id="ui-style-nonce">',
            '<style id="ui-style-nonce" nonce="magnet-test">',
          ),
      );
    await route.fulfill({
      body,
      contentType: asset.endsWith('.js')
        ? 'text/javascript'
        : asset.endsWith('.css')
          ? 'text/css'
          : 'text/html',
      headers: {
        'Content-Security-Policy':
          "default-src 'self'; script-src 'self'; style-src 'self' 'nonce-magnet-test'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'",
      },
    });
  });
  await page.addInitScript(() => {
    const state = {
      tracks: [],
      devices: [],
      device_connection: 'unselected',
      transport_state: 'STOPPED',
      volume: 30,
      duration: 0,
      position: 0,
      queue_mode: 'sequential',
      warnings: [],
    };
    const magnets = { available: true, directory: '/本地下载/magnets', jobs: [], error: '' };
    window.magnetFixture = { magnets, calls: [] };
    window.__TAURI__ = {
      core: {
        invoke: async (command, args) => {
          window.magnetFixture.calls.push({ command, args });
          if (command === 'read_magnet_links') {
            const id = '0123456789abcdef0123456789abcdef01234567';
            return [{ id, title: '测试资源.flac', uri: 'magnet:?xt=urn:btih:' + id }];
          }
          if (command === 'get_config') return { formats: ['mp3'], max_tracks: 20000 };
          if (command === 'get_features')
            return {
              voice: { devices: [], connected: false, listening: false },
              downloads: { jobs: [] },
              magnets,
            };
          if (command === 'add_magnet') {
            const job = {
              id: 'hash',
              title: '测试资源.flac',
              phase: 'resolving',
              total: 0,
              downloaded: 0,
              speed: 0,
              connections: 0,
              error: '',
            };
            magnets.jobs.push(job);
            return job;
          }
          if (command === 'control_magnet') {
            const job = magnets.jobs.find((item) => item.id === args.id);
            job.phase = { pause: 'paused', resume: 'downloading', stop: 'stopped' }[args.action];
            if (args.action === 'resume')
              Object.assign(job, { total: 1024, downloaded: 256, speed: 128, connections: 2 });
            return;
          }
          if (command === 'clear_finished_magnets') {
            magnets.jobs = magnets.jobs.filter(
              (item) => !['stopped', 'completed'].includes(item.phase),
            );
            return;
          }
          return state;
        },
      },
    };
  });
  await page.goto('http://magnet.test/');
  await page.getByRole('button', { name: '磁力链接下载', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: '磁力链接下载', exact: true });
  const field = dialog.getByLabel('磁力链接', { exact: true });
  await expect(field).toBeFocused();
  await expect(field).toHaveValue('magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567');
  expect(
    await page.evaluate(() =>
      window.magnetFixture.calls.some((call) => call.command === 'add_magnet'),
    ),
  ).toBe(false);
  await field.fill('https://example.com');
  await field.press('Enter');
  await expect(dialog.getByRole('alert')).toContainText('请粘贴包含有效 BTIH');
  await expect(field).toBeFocused();
  await field.fill('magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567');
  await dialog.getByRole('button', { name: '开始下载' }).click();
  await expect(field).toHaveValue('');
  await page.keyboard.press('Escape');
  await expect(dialog).not.toBeVisible();
  await expect(page.getByRole('button', { name: '磁力链接下载', exact: true })).toBeFocused();
  const queue = page.getByRole('region', { name: '磁力下载 · 1 个任务' });
  await expect(queue.getByRole('progressbar')).not.toHaveAttribute('value');
  await queue.getByRole('button', { name: '暂停 测试资源.flac' }).click();
  await expect(queue.getByText(/已暂停/)).toBeVisible();
  await queue.getByRole('button', { name: '继续 测试资源.flac' }).click();
  await expect(queue.getByRole('progressbar')).toHaveAttribute('value', '256');
  await queue.getByRole('button', { name: '停止 测试资源.flac' }).click();
  await queue.getByRole('button', { name: '清除结束记录（保留文件）' }).click();
  const confirmation = page.getByRole('dialog', { name: '清除结束记录', exact: true });
  await expect(confirmation.getByRole('button', { name: '取消', exact: true })).toBeFocused();
  await confirmation.getByRole('button', { name: '清除记录', exact: true }).click();
  await expect(queue).not.toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => {
    window.magnetFixture.magnets.available = false;
  });
  await page.getByRole('button', { name: '磁力链接下载', exact: true }).click();
  await expect(dialog.getByText(/尚未找到 aria2/)).toBeVisible();
  await expect(dialog.getByRole('button', { name: '开始下载' })).toBeDisabled();
  await field.fill(
    'magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=' +
      '很长的资源名称'.repeat(30),
  );
  await expect(field).toHaveValue(/很长的资源名称/);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await mkdir('test-results/magnet', { recursive: true });
  await page.screenshot({ path: 'test-results/magnet/narrow.png', fullPage: true });
  expect(errors).toEqual([]);
  console.log(
    'Magnet UI: validation, queue controls, metadata progress, confirmation, focus and narrow missing-engine state passed.',
  );
} finally {
  await browser.close();
}
