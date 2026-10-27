const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const key = 'mobile-session-v1';
const flush = () => new Promise(setImmediate);
function phone({ stored = '', ticket = '', reply, storageFails = false } = {}) {
  const nodes = new Map();
  const node = (id) => {
    if (!nodes.has(id)) nodes.set(id, {
      dataset: {}, attrs: {}, hidden: ['listen-panel', 'send-panel', 'download-form', 'jobs-panel'].includes(id), textContent: '', value: '', disabled: false,
      events: {}, children: [], addEventListener(name, callback) { this.events[name] = callback; },
      setAttribute(k, v) { this.attrs[k] = v; }, focus() {}, replaceChildren(...items) { this.children = items; },
      querySelectorAll() { return []; }, querySelector() { return node(`${id}-submit`); },
      append(...items) { this.children.push(...items); }, scrollIntoView() {},
    });
    return nodes.get(id);
  };
  const storage = new Map(stored ? [[key, stored]] : []);
  const calls = []; const history = []; let tick;
  const library = { player: { mode: 'sequential', connection: 'connected', device_id: 'speaker', device_name: '小爱 Pro', current_track_id: null, transport: 'STOPPED' }, tracks: [], total: 0, library_total: 0, page: 1, pages: 1 };
  let respond = reply || (() => ({ token: 'new-phone-token', jobs: [] }));
  const context = {
    document: { hidden: false, getElementById: node, querySelectorAll: () => [], addEventListener() {}, createElement: node },
    location: { hash: ticket ? `#pair=${ticket}` : '', pathname: '/', search: '' },
    history: { replaceState: (...args) => history.push(args[2]) },
    localStorage: { getItem: (k) => storage.get(k), setItem(k, v) { if (storageFails) throw Error('disabled'); storage.set(k, v); }, removeItem: (k) => storage.delete(k) },
    fetch: async (url, init) => {
      calls.push({ url, body: init.body ? JSON.parse(init.body) : undefined, headers: init.headers, history: [...history] });
      const raw = await respond(url, init);
      const data = url.startsWith('/api/library') && !raw.status ? { ...library, ...raw } : raw;
      return { ok: !data.status, status: data.status || 200, json: async () => data };
    },
    URLSearchParams, AbortController, TypeError, setTimeout, clearTimeout,
    setInterval: (fn) => { tick = fn; },
  };
  vm.runInNewContext(readFileSync('mobile/app.js', 'utf8'), context);
  return { node, storage, calls, history, tick: () => tick(), retry: () => node('connection-retry').events.click(), respond: (fn) => { respond = fn; } };
}

test('QR scan removes ticket from history before automatic pairing and remembers phone', async () => {
  const ui = phone({ ticket: 'one-use-ticket' }); await flush();
  assert.equal(ui.calls[0].url, '/api/pair');
  assert.deepEqual(ui.calls[0].body, { ticket: 'one-use-ticket' });
  assert.deepEqual(ui.calls[0].history, ['/']);
  assert.equal(ui.storage.get(key), 'new-phone-token');
  assert.equal(ui.node('send-panel').hidden, false);
  assert.equal(ui.node('pair-panel').hidden, true);
});
test('bookmark restores stored pairing and a replayed QR does not redeem again', async () => {
  for (const ticket of ['', 'used-ticket']) {
    const ui = phone({ stored: 'saved-token', ticket }); await flush();
    assert.ok(ui.calls.every((c) => c.url === '/api/jobs' || c.url.startsWith('/api/library')));
    assert.equal(ui.calls[0].headers.Authorization, 'Bearer saved-token');
    assert.equal(ui.node('send-panel').hidden, false);
  }
});
test('offline startup retains stored credential and retries without new QR', async () => {
  const ui = phone({ stored: 'saved-token', reply: () => { throw new TypeError('network'); } }); await flush();
  assert.equal(ui.storage.get(key), 'saved-token');
  assert.equal(ui.node('send-panel').hidden, true);
  assert.match(ui.node('connection-error').textContent, /暂时无法连接/);
  ui.respond(() => ({ jobs: [] })); await ui.retry(); await flush();
  assert.equal(ui.node('send-panel').hidden, false);
  assert.equal(ui.node('connection-error').textContent, '');
});
test('revoked pairing is removed; a newly scanned ticket can pair immediately', async () => {
  const ui = phone({ stored: 'old', ticket: 'fresh', reply: (url, init) => init.headers.Authorization === 'Bearer old' ? { status: 401, error: '已忘记手机' } : { token: 'fresh-token', jobs: [] } });
  await flush();
  assert.equal(ui.storage.get(key), 'fresh-token');
  assert.equal(ui.node('send-panel').hidden, false);
  assert.equal(ui.calls.filter((c) => c.url === '/api/pair').length, 1);
});
test('pairing failure stays visible and storage failure does not claim remembered access', async () => {
  const expired = phone({ ticket: 'expired', reply: () => ({ status: 403, error: '二维码已过期' }) }); await flush();
  assert.equal(expired.node('send-panel').hidden, true);
  assert.match(expired.node('connection-error').textContent, /过期/);
  const blocked = phone({ ticket: 'ticket', storageFails: true }); await flush();
  assert.equal(blocked.node('send-panel').hidden, false);
  assert.equal(blocked.storage.size, 0);
  assert.match(blocked.node('remember-note').textContent, /未允许保存/);
});
test('temporary disconnect preserves draft and pairing; revocation hides controls', async () => {
  const ui = phone({ stored: 'saved' }); await flush();
  ui.node('share-text').value = 'my draft';
  ui.respond(() => { throw new TypeError('offline'); }); await ui.tick();
  assert.equal(ui.node('share-text').value, 'my draft');
  assert.equal(ui.storage.get(key), 'saved');
  ui.respond(() => ({ status: 401, error: '请重新扫码' })); await ui.tick();
  assert.equal(ui.storage.size, 0);
  assert.equal(ui.node('send-panel').hidden, true);
  assert.match(ui.node('pair-status').textContent, /失效/);
});

test('phone chooses a library track and sends selected mode without replaying commands', async () => {
  const player = { mode: 'sequential', connection: 'connected', device_id: 'speaker', device_name: '小爱', current_track_id: null, transport: 'STOPPED' };
  const ui = phone({ stored: 'paired', reply: (url, init) => url === '/api/playback' ? { ...player, ...(JSON.parse(init.body).action === 'mode' ? { mode: JSON.parse(init.body).mode } : { current_track_id: 'song', title: '晴天', transport: 'TRANSITIONING' }) } : { jobs: [], ...(url.startsWith('/api/library') ? { player, tracks: [{ id: 'song', title: '晴天', format: 'm4a' }], total: 1, library_total: 1, page: 1, pages: 1 } : {}) } });
  await flush();
  const button = ui.node('phone-tracks').children[0].children[0];
  await button.events.click();
  assert.deepEqual(ui.calls.find((c) => c.url === '/api/playback').body, { action: 'play', track_id: 'song', device_id: 'speaker' });
  ui.node('phone-mode').value = 'repeat-all'; ui.node('phone-mode').events.change();
  await ui.node('mode-form').events.submit({ preventDefault() {} });
  assert.deepEqual(ui.calls.filter((c) => c.url === '/api/playback')[1].body, { action: 'mode', mode: 'repeat-all' });
  assert.equal(ui.calls.filter((c) => c.url === '/api/playback').length, 2);
});
test('phone connection failure disables stale playback controls and polling never sends commands', async () => {
  const ui = phone({ stored: 'paired' }); await flush();
  assert.equal(ui.node('phone-mode').disabled, false);
  ui.respond(() => { throw new TypeError('offline'); }); await ui.tick();
  assert.equal(ui.node('phone-mode').disabled, true);
  assert.equal(ui.node('phone-next').disabled, true);
  assert.equal(ui.calls.filter((c) => c.url === '/api/playback').length, 0);
});

test('pending phone control prevents duplicate dispatch and a mode draft survives polling', async () => {
  let complete;
  const ui = phone({ stored: 'paired', reply: (url) => url === '/api/playback' ? new Promise(resolve => { complete = resolve; }) : { jobs: [] } });
  await flush();
  ui.node('phone-mode').value = 'repeat-one'; ui.node('phone-mode').events.change();
  await ui.tick();
  assert.equal(ui.node('phone-mode').value, 'repeat-one');
  const pending = ui.node('phone-play-all').events.click(); await flush();
  await ui.node('phone-play-all').events.click();
  assert.equal(ui.calls.filter(c => c.url === '/api/playback').length, 1);
  complete({ mode: 'sequential', connection: 'connected', device_id: 'speaker', transport: 'PLAYING' });
  await pending;
  assert.equal(ui.node('phone-play-all').attrs['aria-busy'], 'false');
});
