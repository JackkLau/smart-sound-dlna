import React, { useEffect, useRef, useState } from 'react';

const SESSION_KEY = 'mobile-session-v1';
const REMEMBER_NOTE =
  '收藏此页，或在浏览器菜单选择“添加到主屏幕”。下次直接打开，无需再次扫码；请保持同一 Wi-Fi，并在电脑开启手机接收。';
const NETWORK_ERROR =
  '暂时无法连接电脑。请确认同一 Wi-Fi、电脑应用仍在运行，且“手机点歌 / 导入”已开启。连接恢复后可重试。';
const modes = { sequential: '顺序播放', 'repeat-all': '列表循环', 'repeat-one': '单曲循环' };
const phases = {
  queued: '等待下载',
  resolving: '正在获取音轨',
  downloading: '正在下载',
  importing: '正在加入曲库',
  completed: '已加入电脑曲库',
  failed: '下载未完成',
  cancelling: '正在取消',
  cancelled: '已取消',
};
const initialView = () => ({
  ready: false,
  restoring: false,
  busy: '',
  pairStatus: '正在连接电脑…',
  rememberNote: REMEMBER_NOTE,
  connectionError: '',
  libraryData: null,
  player: null,
  playbackFresh: false,
  libraryLoading: false,
  searchDraft: '',
  page: 1,
  modeDraft: 'sequential',
  modeDirty: false,
  shareText: '',
  preview: null,
  selection: [],
  downloadSummary: '',
  jobs: [],
  errors: { parse: '', download: '', mode: '', listen: '' },
});

function usePhone() {
  const [view, setView] = useState(initialView);
  const model = useRef({
    view,
    active: true,
    token: '',
    ticket: '',
    epoch: 0,
    libraryVersion: 0,
    polling: false,
    restoring: false,
    busy: '',
    query: '',
    page: 1,
    searchComposing: false,
    modeFocused: false,
    formsComposing: new Set(),
    searchTimer: null,
    booted: false,
  });
  const searchRef = useRef(null);
  const shareRef = useRef(null);
  const videoRef = useRef(null);
  const m = model.current;
  function update(patch) {
    m.view = { ...m.view, ...patch };
    if (m.active) setView(m.view);
  }
  function errorAt(key, message) {
    update({ errors: { ...m.view.errors, [key]: message } });
  }
  function connected(value) {
    m.token = value;
    m.epoch += 1;
    m.libraryVersion += 1;
    const patch = { ready: Boolean(value), ...(value ? { connectionError: '' } : {}) };
    try {
      if (value) localStorage.setItem(SESSION_KEY, value);
      else localStorage.removeItem(SESSION_KEY);
    } catch {
      patch.rememberNote =
        '浏览器未允许保存配对。本次可以发送；下次需要在电脑忘记手机后重新扫码。请使用常用浏览器的普通模式。';
    }
    if (!value)
      Object.assign(patch, {
        libraryData: null,
        player: null,
        playbackFresh: false,
        preview: null,
        selection: [],
        jobs: [],
        libraryLoading: false,
      });
    update(patch);
  }
  async function api(path, body) {
    const epoch = m.epoch;
    const requestToken = m.token;
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 120000);
    try {
      const response = await fetch(`/api/${path}`, {
        method: body === undefined ? 'GET' : 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(requestToken ? { Authorization: `Bearer ${requestToken}` } : {}),
        },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        signal: controller.signal,
        cache: 'no-store',
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok) {
        if (
          response.status === 401 &&
          path !== 'pair' &&
          epoch === m.epoch &&
          requestToken === m.token
        ) {
          connected('');
          update({
            pairStatus: '配对已失效，请扫描电脑的新二维码。',
            connectionError: data.error || '配对已失效，请重新扫码。',
          });
        }
        throw new Error(data.error || `电脑未能处理请求（${response.status}），请重试。`);
      }
      return data;
    } catch (error) {
      if (error.name === 'AbortError' || error instanceof TypeError) throw new Error(NETWORK_ERROR);
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }
  async function loadLibrary() {
    if (!m.view.ready || m.busy || m.searchComposing) return;
    const version = ++m.libraryVersion;
    const epoch = m.epoch;
    update({ libraryLoading: true });
    try {
      const data = await api(
        `library?${new URLSearchParams({ q: m.query, page: String(m.page) })}`,
      );
      if (version !== m.libraryVersion || epoch !== m.epoch || !m.view.ready || m.busy || !m.active)
        return;
      m.page = data.page;
      update({
        libraryData: data,
        player: data.player,
        playbackFresh: true,
        page: data.page,
        connectionError: '',
        ...(!m.view.modeDirty && !m.modeFocused ? { modeDraft: data.player.mode } : {}),
      });
    } catch (error) {
      if (version === m.libraryVersion && epoch === m.epoch) throw error;
    } finally {
      if (version === m.libraryVersion && epoch === m.epoch) update({ libraryLoading: false });
    }
  }
  async function poll() {
    if (!m.view.ready || !m.token || m.busy || m.polling || document.hidden || !m.active) return;
    m.polling = true;
    const epoch = m.epoch;
    const version = m.libraryVersion;
    try {
      const data = await api('jobs');
      if (epoch !== m.epoch || version !== m.libraryVersion || m.busy || !m.active) return;
      update({ jobs: data.jobs || [], connectionError: '' });
      await loadLibrary();
    } catch (error) {
      if (epoch === m.epoch) update({ playbackFresh: false, connectionError: error.message });
    } finally {
      m.polling = false;
    }
  }
  async function restore() {
    if (m.restoring || m.busy) return;
    m.restoring = true;
    update({
      restoring: true,
      pairStatus: m.token
        ? '正在恢复与电脑的连接…'
        : m.ticket
          ? '正在自动配对…'
          : '首次使用请扫描电脑“手机点歌 / 导入”中的二维码，无需输入连接码。',
    });
    try {
      if (m.token) {
        try {
          await api('jobs');
          connected(m.token);
          m.ticket = '';
          return;
        } catch (error) {
          if (m.token || !m.ticket) throw error;
        }
      }
      if (m.ticket) {
        const result = await api('pair', { ticket: m.ticket });
        connected(result.token);
        m.ticket = '';
      }
    } catch (error) {
      update({ connectionError: error.message });
    } finally {
      m.restoring = false;
      update({ restoring: false });
      if (m.view.ready) await poll();
    }
  }
  async function action(name, key, callback, field) {
    if (m.busy || m.restoring || !m.view.ready) return;
    m.busy = name;
    m.libraryVersion += 1;
    update({ busy: name, libraryLoading: false });
    errorAt(key, '');
    try {
      await callback();
    } catch (error) {
      errorAt(key, error.message);
      field?.current?.focus();
    } finally {
      m.busy = '';
      update({ busy: '' });
      await poll();
    }
  }
  function submit(name, key, callback, field) {
    return (event) => {
      event.preventDefault();
      if (!m.formsComposing.has(name) && !event.nativeEvent.isComposing)
        return action(name, key, callback, field);
    };
  }
  function formComposition(name) {
    return {
      onCompositionStart: () => m.formsComposing.add(name),
      onCompositionEnd: () => m.formsComposing.delete(name),
    };
  }
  function sendPlayback(name, payload) {
    return action(name, 'listen', async () => {
      const player = m.view.player;
      if (!player?.device_id || player.connection !== 'connected' || !m.view.playbackFresh)
        throw new Error('请先在电脑选择并连接音响。');
      const epoch = m.epoch;
      const next = await api('playback', { ...payload, device_id: player.device_id });
      if (epoch === m.epoch && m.view.ready) update({ player: next, playbackFresh: true });
    });
  }
  function search(value, immediate = false) {
    clearTimeout(m.searchTimer);
    update({ searchDraft: value });
    m.libraryVersion += 1;
    if (m.searchComposing) return;
    m.query = value.trim();
    m.page = 1;
    update({ page: 1 });
    const load = () =>
      loadLibrary().catch((error) => {
        update({ playbackFresh: false });
        errorAt('listen', error.message);
      });
    if (immediate) load();
    else m.searchTimer = setTimeout(load, 300);
  }
  function clearShare() {
    if (m.busy || m.restoring) return;
    m.formsComposing.delete('parse');
    update({
      shareText: '',
      preview: null,
      selection: [],
      downloadSummary: '',
      errors: { ...m.view.errors, parse: '', download: '' },
    });
    shareRef.current?.focus();
  }
  function changePage(delta) {
    m.page += delta;
    update({ page: m.page });
    loadLibrary().catch((error) => {
      update({ playbackFresh: false });
      errorAt('listen', error.message);
    });
  }
  useEffect(() => {
    m.active = true;
    if (!m.booted) {
      m.booted = true;
      m.ticket = new URLSearchParams(location.hash.slice(1)).get('pair') || '';
      // Remove a one-use ticket before the first network request; never persist it.
      if (location.hash) history.replaceState(null, '', location.pathname + location.search);
      try {
        m.token = localStorage.getItem(SESSION_KEY) || '';
      } catch {
        /* In-memory pairing still works. */
      }
    }
    restore();
    const interval = setInterval(poll, 3000);
    const visibility = () => {
      if (!document.hidden) {
        if (m.view.ready) poll();
        else restore();
      }
    };
    document.addEventListener('visibilitychange', visibility);
    return () => {
      m.active = false;
      m.libraryVersion += 1;
      clearInterval(interval);
      clearTimeout(m.searchTimer);
      document.removeEventListener('visibilitychange', visibility);
    };
  }, []);
  useEffect(() => {
    if (view.preview) videoRef.current?.scrollIntoView?.({ block: 'nearest' });
  }, [view.preview?.id]);
  const parseSubmit = submit(
    'parse',
    'parse',
    async () => {
      if (!m.view.shareText.trim()) throw new Error('请先粘贴 Bilibili 视频分享链接。');
      update({ preview: null, selection: [], downloadSummary: '' });
      errorAt('download', '');
      const epoch = m.epoch;
      const preview = await api('parse', { text: m.view.shareText });
      if (epoch !== m.epoch || !m.view.ready) return;
      update({
        preview,
        selection: preview.parts
          .filter((part) => part.page === preview.selected_page)
          .map((part) => part.cid),
      });
    },
    shareRef,
  );
  const downloadSubmit = submit('download', 'download', async () => {
    const { preview, selection } = m.view;
    if (!preview) throw new Error('请先解析视频。');
    if (!selection.length || selection.length > 100) throw new Error('请选择 1 至 100 首音轨。');
    const epoch = m.epoch;
    const cids = preview.parts
      .filter((part) => selection.includes(part.cid))
      .map((part) => part.cid);
    const result = await api('download', { preview_id: preview.id, cids });
    if (epoch !== m.epoch || !m.view.ready) return;
    const accepted = new Set(result.items.filter((item) => item.accepted).map((item) => item.cid));
    update({
      selection: m.view.selection.filter((cid) => !accepted.has(cid)),
      downloadSummary: `已发送 ${accepted.size} 首，电脑将下载并加入曲库。`,
    });
    errorAt(
      'download',
      result.items
        .filter((item) => item.error)
        .map((item) => `${item.title}：${item.error}`)
        .join('\n'),
    );
  });
  const modeSubmit = submit('mode', 'mode', async () => {
    const epoch = m.epoch;
    const player = await api('playback', { action: 'mode', mode: m.view.modeDraft });
    if (epoch === m.epoch && m.view.ready)
      update({ player, modeDirty: false, modeDraft: player.mode });
  });
  return {
    view,
    update,
    m,
    restore,
    poll,
    search,
    searchRef,
    shareRef,
    videoRef,
    parseSubmit,
    downloadSubmit,
    modeSubmit,
    formComposition,
    sendPlayback,
    changePage,
    clearShare,
  };
}

export default function MobileApp() {
  const phone = usePhone();
  const { view: v, update, m, searchRef, shareRef, videoRef } = phone;
  const player = v.player;
  const library = v.libraryData;
  const busy = Boolean(v.busy || v.restoring);
  const usable = v.ready && v.playbackFresh && Boolean(player) && !busy;
  const speaker = usable && player.connection === 'connected' && Boolean(player.device_id);
  const hasMusic = Boolean(library?.library_total);
  const transportLabels = {
    PLAYING: '正在播放',
    PAUSED_PLAYBACK: '已暂停',
    TRANSITIONING: '正在准备播放',
    STOPPED: '已停止',
    NO_MEDIA_PRESENT: '已停止',
  };
  const speakerStatus = !player
    ? '正在读取音响状态…'
    : player.connection === 'connected'
      ? `${player.device_name} · ${transportLabels[player.transport] || '等待播放'}`
      : player.connection === 'disconnected'
        ? '音响已断开，电脑正在重连'
        : '请先在电脑选择并连接音响';
  const toggleLabel =
    player?.transport === 'PLAYING' && player.current_track_id
      ? '暂停'
      : player?.transport === 'PAUSED_PLAYBACK' && player.current_track_id
        ? '继续播放'
        : '播放';
  const libraryStatus = !library
    ? v.libraryLoading
      ? '正在读取曲库…'
      : '等待连接电脑曲库…'
    : library.total
      ? `共 ${library.total} 首 · 当前第 ${(v.page - 1) * 20 + 1}–${Math.min(v.page * 20, library.total)} 首`
      : m.query
        ? '没有找到歌曲，请换一个曲名或清空搜索。'
        : '曲库为空，请先在电脑添加音乐，或在下方导入视频音轨。';
  const error = (key) => (
    <p id={`${key}-error`} className="field-error" role="alert">
      {v.errors[key]}
    </p>
  );
  const control = (command, label) => (
    <button
      disabled={
        !speaker ||
        (command === 'stop'
          ? !player?.current_track_id || ['STOPPED', 'NO_MEDIA_PRESENT'].includes(player?.transport)
          : !hasMusic)
      }
      id={`phone-${command}`}
      type="button"
      className="button outline"
      aria-busy={v.busy === command}
      onClick={() => phone.sendPlayback(command, { action: 'control', command })}
    >
      {label}
    </button>
  );
  return (
    <>
      <header className="mobile-heading">
        <span className="eyebrow">小米音响 · 本地音乐</span>
        <h1>手机点歌</h1>
        <p>选择电脑曲库中的歌曲，由电脑推送到音响。也可以发送 Bilibili 视频，下载音轨到电脑。</p>
      </header>
      <main>
        <section id="pair-panel" aria-labelledby="pair-title" hidden={v.ready}>
          <h2 id="pair-title">连接电脑</h2>
          <p id="pair-status" className="field-help" role="status">
            {v.pairStatus}
          </p>
          <button
            disabled={busy}
            id="connection-retry"
            className="button outline"
            type="button"
            aria-busy={v.restoring}
            onClick={phone.restore}
          >
            重试连接
          </button>
        </section>
        <section
          id="listen-panel"
          hidden={!v.ready}
          aria-labelledby="listen-title"
          aria-busy={v.libraryLoading}
        >
          <h2 id="listen-title">电脑音乐库</h2>
          <p id="speaker-status" className="field-help" role="status">
            {speakerStatus}
          </p>
          <strong id="playing-title">{player?.title || '还没有播放本地音乐'}</strong>
          <form
            id="mode-form"
            noValidate
            onSubmit={phone.modeSubmit}
            {...phone.formComposition('mode')}
          >
            <label className="field-label" htmlFor="phone-mode">
              播放模式
            </label>
            <select
              disabled={!usable}
              id="phone-mode"
              className="text-input"
              aria-describedby="mode-error"
              value={v.modeDraft}
              onFocus={() => {
                m.modeFocused = true;
              }}
              onBlur={() => {
                m.modeFocused = false;
              }}
              onChange={(event) => update({ modeDraft: event.target.value, modeDirty: true })}
            >
              {Object.entries(modes).map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
            <button
              disabled={!usable}
              type="submit"
              className="button outline"
              aria-busy={v.busy === 'mode'}
            >
              应用播放模式
            </button>
            {error('mode')}
          </form>
          <p id="phone-queue-help" className="field-help">
            {player
              ? `${modes[player.mode]} · 按电脑完整曲库排列，搜索只用于找歌。${v.modeDirty ? '新模式尚未应用。' : ''}`
              : '顺序按电脑的完整曲库排列，搜索只用于找歌。'}
          </p>
          <div id="phone-transport" className="mobile-transport" aria-label="音响控制">
            {control('previous', '上一首')}
            <button
              disabled={!speaker || !hasMusic || player.transport === 'TRANSITIONING'}
              id="phone-toggle"
              type="button"
              className="button primary"
              aria-busy={v.busy === 'toggle'}
              onClick={() => {
                const command =
                  player?.transport === 'PLAYING'
                    ? 'pause'
                    : player?.transport === 'PAUSED_PLAYBACK'
                      ? 'resume'
                      : null;
                phone.sendPlayback(
                  'toggle',
                  command && player?.current_track_id
                    ? { action: 'control', command }
                    : player?.current_track_id
                      ? { action: 'play', track_id: player.current_track_id }
                      : { action: 'play-all' },
                );
              }}
            >
              {toggleLabel}
            </button>
            {control('next', '下一首')}
            {control('stop', '停止')}
          </div>
          <button
            disabled={!speaker || !hasMusic}
            id="phone-play-all"
            type="button"
            className="button outline"
            aria-busy={v.busy === 'play-all'}
            onClick={() => phone.sendPlayback('play-all', { action: 'play-all' })}
          >
            播放全部 · 从第一首顺序播放
          </button>
          <p id="listen-error" className="field-error" role="alert">
            {v.errors.listen}
          </p>
          <label className="field-label" htmlFor="phone-search">
            搜索曲名
          </label>
          <div className="mobile-search">
            <input
              ref={searchRef}
              id="phone-search"
              type="search"
              className="text-input"
              maxLength={120}
              placeholder="搜索电脑曲库"
              autoComplete="off"
              disabled={busy}
              value={v.searchDraft}
              onCompositionStart={() => {
                m.searchComposing = true;
                m.libraryVersion += 1;
                clearTimeout(m.searchTimer);
              }}
              onCompositionEnd={(event) => {
                m.searchComposing = false;
                phone.search(event.currentTarget.value);
              }}
              onChange={(event) => phone.search(event.target.value)}
            />
            <button
              disabled={busy}
              hidden={!v.searchDraft}
              id="phone-clear"
              type="button"
              className="button outline"
              aria-label="清空搜索曲名"
              onClick={() => {
                phone.search('', true);
                searchRef.current?.focus();
              }}
            >
              清空
            </button>
          </div>
          <p id="library-status" className="field-help" role="status">
            {libraryStatus}
          </p>
          <ul id="phone-tracks" className="mobile-tracks" aria-label="电脑曲库中的音乐">
            {(library?.tracks || []).map((track) => (
              <li key={track.id}>
                <button
                  type="button"
                  className="mobile-track"
                  data-track-id={track.id}
                  aria-current={track.id === player?.current_track_id ? 'true' : 'false'}
                  aria-label={`播放 ${track.title}`}
                  disabled={!speaker}
                  onClick={() =>
                    phone.sendPlayback(`track-${track.id}`, { action: 'play', track_id: track.id })
                  }
                >
                  <strong>{track.title}</strong>
                  <span>
                    {track.format.toUpperCase()} ·{' '}
                    {track.id === player?.current_track_id ? '当前歌曲' : '点击播放'}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          <div className="mobile-pagination">
            <button
              disabled={!usable || !library || v.page <= 1}
              id="phone-page-previous"
              type="button"
              className="button outline"
              onClick={() => phone.changePage(-1)}
            >
              上一页
            </button>
            <span id="phone-page-info">
              {v.page} / {library?.pages || 1}
            </span>
            <button
              disabled={!usable || !library || v.page >= library.pages}
              id="phone-page-next"
              type="button"
              className="button outline"
              onClick={() => phone.changePage(1)}
            >
              下一页
            </button>
          </div>
          <button
            disabled={busy || v.libraryLoading}
            id="library-retry"
            type="button"
            className="button outline"
            onClick={phone.poll}
          >
            刷新曲库与音响状态
          </button>
          <p className="field-help">
            先在电脑选择并连接音响。点击歌曲会立即播放，并按已应用的模式继续；电脑应用需保持运行。
          </p>
        </section>
        <section id="send-panel" hidden={!v.ready} aria-labelledby="send-title">
          <div className="mobile-section-heading">
            <h2 id="send-title">选择视频音轨</h2>
            <span className="field-help">已连接电脑</span>
          </div>
          <p id="remember-note" className="field-help">
            {v.rememberNote}
          </p>
          <form
            id="parse-form"
            noValidate
            onSubmit={phone.parseSubmit}
            {...phone.formComposition('parse')}
          >
            <label className="field-label" htmlFor="share-text">
              视频链接或分享文案
            </label>
            <div className="mobile-input-field">
              <textarea
                ref={shareRef}
                className="text-input resize-none"
                id="share-text"
                rows={3}
                maxLength={8192}
                required
                placeholder="粘贴 Bilibili 分享内容，支持 b23.tv、BV 号和分 P 链接"
                aria-describedby="parse-error"
                aria-invalid={Boolean(v.errors.parse)}
                value={v.shareText}
                disabled={busy}
                onChange={(event) => update({ shareText: event.target.value })}
              />
              {v.shareText && (
                <button
                  id="share-clear"
                  type="button"
                  className="text-button mobile-input-clear"
                  aria-label="清空视频链接或分享文案"
                  aria-controls="share-text"
                  disabled={busy}
                  onClick={phone.clearShare}
                >
                  清空
                </button>
              )}
            </div>
            <button
              className="button primary"
              type="submit"
              disabled={busy}
              aria-busy={v.busy === 'parse'}
            >
              发送并解析
            </button>
            {error('parse')}
          </form>
          <form
            id="download-form"
            hidden={!v.preview}
            noValidate
            onSubmit={phone.downloadSubmit}
            {...phone.formComposition('download')}
          >
            <h3 id="video-title" ref={videoRef}>
              {v.preview?.title}
            </h3>
            <p id="video-author" className="field-help">
              {v.preview ? `${v.preview.author} · ${v.preview.parts.length} 个音轨` : ''}
            </p>
            <p className="field-help">
              选择要保存的音轨，每次最多 100 首。多 P 视频默认选中链接指定的那一 P。
            </p>
            <fieldset id="parts" disabled={busy}>
              <legend className="field-label">下载哪些音轨</legend>
              {(v.preview?.parts || []).map((part) => (
                <label key={part.cid} className="mobile-part">
                  <input
                    type="checkbox"
                    value={part.cid}
                    checked={v.selection.includes(part.cid)}
                    onChange={(event) =>
                      update({
                        selection: event.target.checked
                          ? [...v.selection, part.cid]
                          : v.selection.filter((cid) => cid !== part.cid),
                      })
                    }
                  />
                  <span>
                    P{part.page} · {part.title}
                  </span>
                </label>
              ))}
            </fieldset>
            <p id="selected-count" className="field-help" role="status">
              已选 {v.selection.length} 首（最多 100 首）
            </p>
            <button
              className="button primary"
              type="submit"
              disabled={busy}
              aria-busy={v.busy === 'download'}
            >
              在电脑下载选中音轨
            </button>
            {error('download')}
            <p id="download-summary" role="status">
              {v.downloadSummary}
            </p>
          </form>
        </section>
        <section id="jobs-panel" hidden={!v.ready || !v.jobs.length} aria-labelledby="jobs-title">
          <h2 id="jobs-title">电脑下载进度</h2>
          <p className="field-help">关闭手机页面后，电脑会继续下载。取消或重试请在电脑操作。</p>
          <ul id="jobs" className="mobile-jobs">
            {v.jobs.map((job) => (
              <li key={job.id}>
                <strong>{job.title}</strong>
                <p>
                  {phases[job.phase] || job.phase}
                  {job.phase === 'downloading' && job.total
                    ? ` · ${Math.floor((job.downloaded / job.total) * 100)}%`
                    : ''}
                </p>
                {job.error && <p className="field-error">{job.error}</p>}
              </li>
            ))}
          </ul>
        </section>
        <p id="connection-error" className="field-error" role="alert">
          {v.connectionError}
        </p>
        <p className="field-help mobile-note">
          仅下载你有权保存的内容。请保持电脑应用运行；需要账号权限的视频，请在电脑登录
          Bilibili。此入口使用本地 HTTP，请仅在可信的家庭 Wi-Fi 使用。
        </p>
      </main>
    </>
  );
}
