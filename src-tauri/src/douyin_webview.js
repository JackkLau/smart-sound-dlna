(() => {
  'use strict';
  if (window.top !== window) return;
  const expected = __EXPECTED_ID__;
  const callback = __CALLBACK_PATH__;
  let done = false;
  const accept = (item) => {
    if (done || !item || String(item.aweme_id) !== expected || item.images?.length) return;
    const urls = item.video?.play_addr?.url_list;
    if (!Array.isArray(urls) || !urls.length) return;
    done = true;
    const result = { id: expected, title: String(item.desc || '').slice(0, 600),
      author: String(item.author?.nickname || '抖音作者').slice(0, 200),
      duration: Math.floor(Number(item.video?.duration || 0) / 1000),
      urls: urls.filter(u => typeof u === 'string' && u.length < 8192).slice(0, 8) };
    location.replace('https://douyin-import.invalid' + callback + '?data=' + encodeURIComponent(JSON.stringify(result)));
  };
  const inspect = (data) => {
    if (!data || typeof data !== 'object') return;
    accept(data.aweme_detail);
    accept(data.data?.aweme_detail);
    for (const item of data.item_list || []) accept(item);
    for (const item of data.aweme_list || []) accept(item);
  };
  const relevant = (raw) => {
    try {
      const url = new URL(raw, location.href);
      return ['www.douyin.com', 'www.iesdouyin.com'].includes(url.hostname) &&
        /\/aweme\/(detail|iteminfo)|\/aweme\/v1\/web\/aweme\//.test(url.pathname);
    } catch { return false; }
  };
  const originalFetch = window.fetch;
  window.fetch = function(...args) {
    const response = originalFetch.apply(this, args);
    if (relevant(typeof args[0] === 'string' ? args[0] : args[0]?.url))
      response.then(r => r.clone().text()).then(t => { if (t.length < 2000000) inspect(JSON.parse(t)); }).catch(() => {});
    return response;
  };
  const open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function(method, url, ...args) {
    if (relevant(url)) this.addEventListener('load', () => {
      try {
        if (this.responseType === 'json') inspect(this.response);
        else if ((!this.responseType || this.responseType === 'text') && this.responseText.length < 2000000) inspect(JSON.parse(this.responseText));
      } catch { /* Public page may return an access challenge, never solve it. */ }
    }, { once: true });
    return open.call(this, method, url, ...args);
  };
  // Use the same public, read-only detail request from the loaded official
  // origin. Its own browser session remains inside this disposable webview.
  const refresh = () => {
    if (!done && location.hostname === 'www.douyin.com')
      window.fetch('/aweme/v1/web/aweme/detail/?aweme_id=' + expected, { credentials: 'same-origin' })
        .then(r => r.text()).then(t => { if (t.length < 2000000) inspect(JSON.parse(t)); }).catch(() => {});
  };
  setTimeout(refresh, 4000);
  setTimeout(refresh, 18000);
  // Some share-page deployments ship the detail in SSR instead of an API call.
  const timer = setInterval(() => {
    if (done) { clearInterval(timer); return; }
    for (const loader of Object.values(window._ROUTER_DATA?.loaderData || {})) inspect(loader?.videoInfoRes);
    const render = document.getElementById('RENDER_DATA');
    if (render?.textContent && render.textContent.length < 2000000) {
      try {
        const scan = (value, depth = 0) => {
          if (!value || typeof value !== 'object' || depth > 12) return;
          accept(value);
          for (const child of Object.values(value)) scan(child, depth + 1);
        };
        scan(JSON.parse(decodeURIComponent(render.textContent)));
      } catch { /* Not a supported SSR representation. */ }
    }
    // Keep the preview silent. This does not request playback or dismiss login.
    document.querySelectorAll('video').forEach(video => { video.muted = true; });
  }, 400);
  setTimeout(() => clearInterval(timer), 120000);
})();
