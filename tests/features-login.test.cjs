const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');

// A minimal DOM boundary for the existing plain-JS controller. The test runs
// the production controller and drives the same get_features snapshots as IPC.
function harness() {
  const nodes = new Map();
  function node(id) {
    if (!nodes.has(id)) nodes.set(id, {
      id, textContent: '', hidden: false, disabled: false, value: '', dataset: {},
      addEventListener() {}, setAttribute() {}, replaceChildren() {}, focus() {},
    });
    return nodes.get(id);
  }
  const accounts = Object.fromEntries(['xiaomi', 'bilibili'].map((provider) => {
    const buttons = Object.fromEntries(['login', 'finish-login', 'disconnect'].map((action) => [action, node(`${provider}-${action}`)]));
    return [provider, {
      querySelector: (selector) => buttons[selector.slice(6, -1)],
      querySelectorAll: () => Object.values(buttons),
    }];
  }));
  let tick;
  let data = {
    voice: { connected: false, listening: false, devices: [] },
    downloads: { authenticated: false, directory: '', jobs: [] },
    xiaomi_login: { phase: 'idle' }, bilibili_login: { phase: 'idle' },
  };
  const announcements = [];
  const context = {
    document: {
      getElementById: node,
      querySelectorAll: () => [],
      querySelector: (selector) => accounts[selector.match(/data-provider="(.*?)"/)[1]],
    },
    window: { MusicUI: {
      invoke: async () => data, announce: (message) => announcements.push(message),
      refresh() {}, messageOf: (e) => String(e), formatSize: String, formatTime: String,
      element: (tag, _className, text) => ({ tag, textContent: text }),
    } },
    setInterval: (callback) => { tick = callback; },
  };
  vm.runInNewContext(readFileSync('frontend/features.js', 'utf8'), context);
  return {
    node, announcements,
    async update(phase, connected = false, error) {
      await new Promise(setImmediate);
      data = { ...data, voice: { ...data.voice, connected }, xiaomi_login: { phase, error } };
      await tick();
    },
  };
}

test('automatic login replaces waiting UI, hides retry and announces success once', async () => {
  const ui = harness();
  await ui.update('waiting');
  assert.match(ui.node('xiaomi-account-status').textContent, /自动关闭/);
  assert.equal(ui.node('xiaomi-finish-login').hidden, false);
  await ui.update('checking');
  assert.match(ui.node('xiaomi-account-status').textContent, /正在确认/);
  assert.equal(ui.node('xiaomi-finish-login').disabled, true);
  await ui.update('connected', true);
  assert.match(ui.node('xiaomi-account-status').textContent, /^已连接/);
  assert.equal(ui.node('xiaomi-finish-login').hidden, true);
  assert.equal(ui.node('xiaomi-disconnect').hidden, false);
  assert.equal(ui.node('voice-badge').textContent, '已连接');
  await ui.update('connected', true);
  assert.equal(ui.announcements.length, 1);
});

test('cancelled window clears waiting status and retry controls', async () => {
  const ui = harness();
  await ui.update('waiting');
  await ui.update('cancelled');
  assert.match(ui.node('xiaomi-account-status').textContent, /登录已取消/);
  assert.equal(ui.node('xiaomi-finish-login').hidden, true);
  assert.equal(ui.node('xiaomi-disconnect').hidden, true);
  assert.equal(ui.node('voice-badge').textContent, '未连接');
});

test('provider error stays visible until retry succeeds', async () => {
  const ui = harness();
  await ui.update('failed', false, '小米服务连接失败，请检查网络后重试。');
  assert.match(ui.node('xiaomi-login-error').textContent, /连接失败/);
  assert.equal(ui.node('xiaomi-finish-login').hidden, false);
  assert.equal(ui.node('xiaomi-finish-login').disabled, false);
  await ui.update('connected', true);
  assert.equal(ui.node('xiaomi-login-error').textContent, '');
});
