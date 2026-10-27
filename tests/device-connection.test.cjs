const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');

// Execute the real app controller against its DOM/IPC boundary. The fixture
// supplies speaker states but does not reimplement the UI's decisions.
function harness() {
  class Node {
    constructor(tag = 'div') {
      this.tag = tag; this.children = []; this.dataset = {}; this.attrs = {};
      this.handlers = {}; this.value = ''; this.className = ''; this.textContent = '';
      this.classList = { toggle() {}, add() {}, remove() {} };
    }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = children; }
    addEventListener(event, fn) { this.handlers[event] = fn; }
    setAttribute(key, value) { this.attrs[key] = value; }
    getAttribute(key) { return this.attrs[key]; }
    removeAttribute(key) { delete this.attrs[key]; }
    focus() { document.activeElement = this; }
    querySelectorAll(selector) {
      const matches = (n) => selector.split(',').some((part) => {
        part = part.trim();
        return part.startsWith('.') ? n.className.split(' ').includes(part.slice(1)) : n.tag === part;
      });
      return this.children.flatMap((n) => [ ...(matches(n) ? [n] : []), ...n.querySelectorAll(selector) ]);
    }
    querySelector(selector) {
      const found = this.querySelectorAll(selector)[0];
      if (found) return found;
      const child = new Node(selector); this.append(child); return child;
    }
  }
  const nodes = new Map();
  const node = (id) => { if (!nodes.has(id)) nodes.set(id, new Node()); return nodes.get(id); };
  const document = {
    getElementById: node, querySelector: node,
    querySelectorAll: (s) => [...nodes.values()].flatMap((n) => n.querySelectorAll(s)),
    createElement: (tag) => new Node(tag), createElementNS: (_, tag) => new Node(tag),
    createTextNode: (text) => Object.assign(new Node('text'), { textContent: text }),
  };
  let data = {
    tracks: [{ id: 'song', title: 'Test music', filename: 'music.mp3', format: 'mp3', size: 10 }],
    devices: [{ id: 'speaker', name: 'Test speaker', address: '192.168.1.5' }],
    selected_device_id: 'speaker', device_connection: 'checking', current_track_id: null,
    transport_state: 'STOPPED', discovering: false, status_error: null,
  };
  const calls = [];
  const edited = [], removed = [];
  let tick;
  vm.runInNewContext(readFileSync('frontend/app.js', 'utf8'), {
    document, window: {
      MusicTrackEditor: { open: (track) => edited.push(track.id) },
      MusicLibraryActions: { openRemove: (track) => removed.push(track.id) },
      __TAURI__: { core: { invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'get_config') return { formats: ['mp3'] };
      if (command === 'select_device') data = { ...data, device_connection: 'checking' };
      return data;
    } } } },
    sessionStorage: { getItem: () => null, setItem() {} },
    setTimeout() {}, clearTimeout() {}, setInterval: (fn) => { tick = fn; },
  });
  return {
    node, calls, edited, removed,
    get trackButton() { return node('tracks').querySelector('button'); },
    get deviceButton() { return node('devices').querySelector('button'); },
    async update(patch) {
      await new Promise(setImmediate);
      data = { ...data, ...patch };
      await tick();
    },
  };
}

test('only confirmed speaker connections enable playback; reconnect updates existing UI', async () => {
  const ui = harness();
  await ui.update({ device_connection: 'checking' });
  assert.equal(ui.trackButton.disabled, true);
  assert.match(ui.node('device-hint').textContent, /确认音响连接/);
  await ui.update({ device_connection: 'connected' });
  assert.equal(ui.trackButton.disabled, false);
  assert.equal(ui.node('play-pause').disabled, false);
  assert.match(ui.node('device-hint').textContent, /手动搜索/);
  assert.match(ui.node('discover').querySelector('span').textContent, /搜索其他音响/);
  await ui.deviceButton.handlers.click();
  assert.equal(ui.calls.filter((c) => c.command === 'select_device').length, 0);
  await ui.update({ device_connection: 'disconnected', transport_state: 'UNAVAILABLE' });
  assert.equal(ui.trackButton.disabled, true);
  assert.equal(ui.node('play-pause').disabled, true);
  assert.match(ui.node('device-hint').textContent, /自动重连/);
  assert.match(ui.node('now-subtitle').textContent, /已断开/);
  assert.match(ui.deviceButton.getAttribute('aria-label'), /点击重试/);
  await ui.deviceButton.handlers.click();
  assert.equal(ui.calls.filter((c) => c.command === 'select_device').length, 1);
  assert.match(ui.node('device-hint').textContent, /确认音响连接/);
  await ui.update({ device_connection: 'connected', transport_state: 'STOPPED' });
  assert.equal(ui.trackButton.disabled, false);
  assert.equal(ui.node('play-pause').disabled, false);
  assert.equal(ui.calls.filter((c) => c.command === 'play_track' || c.command === 'control').length, 0);
});

test('the row has one native play action and edit/remove never trigger playback', async () => {
  const ui = harness();
  await ui.update({ device_connection: 'connected' });
  const row = ui.node('tracks').children[0];
  const play = row.querySelector('.track-play');
  assert.equal(play.tag, 'button');
  assert.equal(play.getAttribute('aria-label'), '播放 Test music');
  await row.querySelector('.track-edit').handlers.click();
  await row.querySelector('.track-remove').handlers.click();
  assert.deepEqual(ui.edited, ['song']);
  assert.deepEqual(ui.removed, ['song']);
  assert.equal(ui.calls.filter((c) => c.command === 'play_track').length, 0);
  await play.handlers.click();
  assert.equal(ui.calls.filter((c) => c.command === 'play_track').length, 1);
  await ui.update({ device_connection: 'disconnected' });
  assert.equal(ui.trackButton.disabled, true);
  assert.equal(ui.node('tracks').querySelector('.track-remove').disabled, false);
});

test('empty discovery offers a manual search and status polling does not trigger discovery', async () => {
  const ui = harness();
  for (let i = 0; i < 20; i += 1) {
    await ui.update({ devices: [], selected_device_id: null, device_connection: 'unselected' });
  }
  assert.match(ui.node('device-hint').textContent, /可重新搜索或手动添加/);
  assert.equal(ui.calls.filter((c) => c.command === 'discover_devices').length, 0);
  await ui.node('discover').handlers.click();
  assert.equal(ui.calls.filter((c) => c.command === 'discover_devices').length, 1);
});

test('play-all is one atomic command and queue help explains the next track and final boundary', async () => {
  const ui = harness();
  const tracks = [{ id: 'a', title: 'First', filename: 'a.mp3', format: 'mp3', size: 1 }, { id: 'b', title: 'Second', filename: 'b.mp3', format: 'mp3', size: 1 }];
  await ui.update({ device_connection: 'connected', tracks, current_track_id: 'a', queue_mode: 'sequential' });
  assert.match(ui.node('queue-help').textContent, /下一首：Second/);
  await ui.node('play-all').handlers.click();
  assert.deepEqual(JSON.parse(JSON.stringify(ui.calls.filter(c => c.command === 'control').at(-1).args)), { action: 'play-all', value: null });
  await ui.update({ current_track_id: 'b' });
  assert.match(ui.node('queue-help').textContent, /最后一首，播完停止/);
  await ui.update({ queue_mode: 'repeat-all' });
  assert.match(ui.node('queue-help').textContent, /下一首：First/);
});
