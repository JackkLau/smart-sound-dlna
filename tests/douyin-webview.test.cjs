const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('src-tauri/src/douyin_webview.js', 'utf8');
const id = '7518247213001592121';
function harness(data) {
  const calls = [], callbacks = [], timers = [];
  const window = { fetch: async (...args) => {
    calls.push(args);
    const text = async () => JSON.stringify(data);
    return { text, clone: () => ({ text }) };
  } };
  window.top = window;
  const context = { window, location: { hostname: 'www.douyin.com', href: `https://www.douyin.com/video/${id}`, replace: url => callbacks.push(url) },
    document: { querySelectorAll: () => [], getElementById: () => null },
    URL, XMLHttpRequest: class { open() {} }, setTimeout: (fn, delay) => timers.push({ fn, delay }), setInterval: () => 1, clearInterval() {}, encodeURIComponent, decodeURIComponent };
  vm.runInNewContext(source.replace('__EXPECTED_ID__', JSON.stringify(id)).replace('__CALLBACK_PATH__', JSON.stringify('/test-nonce')), context);
  return { window, calls, callbacks, timers };
}
const flush = async () => { for (let n = 0; n < 12; n++) await Promise.resolve(); };
const detail = () => ({ aweme_detail: { aweme_id: id, desc: '歌手《曲名》<script>test</script>', author: { nickname: '作者' }, video: { duration: 330800, play_addr: { url_list: ['https://v3.douyinvod.com/audio.mp4'] } } } });
test('official page requests only its public detail and exports bounded matching media once', async () => {
  const h = harness(detail()); h.timers.find(t => t.delay === 4000).fn(); await flush();
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0][0], `/aweme/v1/web/aweme/detail/?aweme_id=${id}`);
  assert.equal(h.calls[0][1].credentials, 'same-origin');
  assert.equal(h.callbacks.length, 1);
  const callback = new URL(h.callbacks[0]); assert.equal(callback.host, 'douyin-import.invalid'); assert.equal(callback.pathname, '/test-nonce');
  const result = JSON.parse(callback.searchParams.get('data'));
  assert.deepEqual(Object.keys(result).sort(), ['author','duration','id','title','urls']);
  assert.equal(result.id,id); assert.equal(result.duration,330); assert.equal(result.title, detail().aweme_detail.desc);
  h.timers.find(t => t.delay === 18000).fn(); await flush(); assert.equal(h.callbacks.length,1);
});
test('other videos, slideshows, denied responses and unrelated account requests cannot complete a preview', async () => {
  for (const data of [{}, { status_code: 403 }, { aweme_detail: { ...detail().aweme_detail, aweme_id: '7518247213001592122' } }, { aweme_detail: { ...detail().aweme_detail, images: [{}] } }]) {
    const h = harness(data); h.timers.find(t => t.delay === 4000).fn(); await flush(); assert.equal(h.callbacks.length,0);
  }
  const h = harness(detail()); await h.window.fetch('/account/private'); await flush(); assert.equal(h.callbacks.length,0);
});
