import { useEffect, useRef, useState } from 'react';
import { Dialog, Icon } from './components.jsx';
import { formatSize, messageOf } from './utils.js';
import { useDesktop } from './useDesktop.js';
import { TrackDialogs } from './TrackDialogs.jsx';
import Player from './Player.jsx';
import FeaturePanels from './features/FeaturePanels.jsx';

function savedView() {
  try {
    const view = JSON.parse(sessionStorage.getItem('music-library-view') || '{}');
    return {
      query: typeof view.query === 'string' ? view.query : '',
      page: Math.max(1, Number.parseInt(view.page, 10) || 1),
    };
  } catch {
    return { query: '', page: 1 };
  }
}
export default function App({ invoke }) {
  const ui = useDesktop(invoke);
  const {
    state,
    config,
    loaded,
    connected,
    available,
    connecting,
    connectionError,
    busy,
    discoveryPending,
    importPending,
    importStatus,
    actionError,
    slowMessage,
    toast,
    announce,
    refresh,
    connect,
    mutate,
    importMusic,
    discover,
  } = ui;
  const [view, setView] = useState(savedView);
  const [search, setSearch] = useState(view.query);
  const [panel, setPanel] = useState(null);
  const [activeTrack, setActiveTrack] = useState(null);
  const [voiceBadge, setVoiceBadge] = useState('未连接');
  const [queueTarget, setQueueTarget] = useState(null);
  const [url, setUrl] = useState('');
  const [deviceError, setDeviceError] = useState('');
  const inputRef = useRef(null);
  const urlRef = useRef(null);
  const libraryTitle = useRef(null);
  const trackButtons = useRef(new Map());
  const composing = useRef(false);
  const deviceComposing = useRef(false);
  const devicePending = useRef(false);
  const query = view.query;
  const normalized = query.toLocaleLowerCase('zh-CN').trim();
  const filtered = state.tracks.filter((track) =>
    `${track.title || ''} ${track.filename || ''}`.toLocaleLowerCase('zh-CN').includes(normalized),
  );
  const pages = Math.max(1, Math.ceil(filtered.length / 20));
  const page = loaded ? Math.min(view.page, pages) : view.page;
  const tracks = filtered.slice((page - 1) * 20, page * 20);
  useEffect(() => {
    if (!loaded) return;
    try {
      sessionStorage.setItem('music-library-view', JSON.stringify({ query, page }));
    } catch {
      /* Keep a usable in-memory view when storage is blocked. */
    }
    if (view.page !== page) setView((current) => ({ ...current, page }));
  }, [loaded, query, page, view.page]);
  const reason = !state.selected_device_id
    ? '先连接音响，再播放音乐'
    : state.device_connection === 'disconnected'
      ? '音响已断开，正在自动重连'
      : state.device_connection !== 'connected'
        ? '正在确认音响连接…'
        : '';
  const usable = Boolean(connected && config);
  const blocked = !usable || busy || importPending;
  const ready = !blocked && !reason;
  const device = state.devices.find((item) => item.id === state.selected_device_id);
  const devices = [...state.devices].sort(
    (a, b) =>
      Number(Boolean(b.is_xiaomi_pro)) - Number(Boolean(a.is_xiaomi_pro)) ||
      Number(Boolean(b.is_xiaomi)) - Number(Boolean(a.is_xiaomi)),
  );
  const discovering = Boolean(state.discovering || discoveryPending);
  const proCount = state.devices.filter((item) => item.is_xiaomi_pro).length;
  const deviceHint = !available
    ? '通过桌面客户端自动搜索音响'
    : connectionError
      ? '客户端未就绪，暂时无法搜索'
      : state.device_connection === 'disconnected'
        ? '连接已断开，正在自动重连；点击当前音响可立即重试'
        : discovering
          ? discoveryPending
            ? '正在重新搜索局域网音响…'
            : '正在自动发现局域网音响…'
          : state.device_connection === 'checking'
            ? '正在确认音响连接…'
            : state.device_connection === 'connected'
              ? '已连接 · 需要更换设备时手动搜索'
              : state.discovery_error
                ? '搜索未完成，可重新搜索或手动添加'
                : devices.length
                  ? proCount
                    ? `发现 ${devices.length} 台音响，含 ${proCount} 台小爱音箱 Pro`
                    : `发现 ${devices.length} 台音响，点击即可连接`
                  : loaded
                    ? '暂未发现音响，可重新搜索或手动添加'
                    : '启动后自动发现局域网音响';
  const currentIndex = state.tracks.findIndex((track) => track.id === state.current_track_id);
  const current = state.tracks[currentIndex];
  const next =
    state.queue_mode === 'repeat-one'
      ? current
      : state.tracks[currentIndex + 1] ||
        (state.queue_mode === 'repeat-all' ? state.tracks[0] : null);
  const modeName =
    { sequential: '顺序播放', 'repeat-all': '列表循环', 'repeat-one': '单曲循环' }[
      state.queue_mode
    ] || '顺序播放';
  const errors = [
    ...new Set(
      [actionError, slowMessage, state.status_error, ...(state.warnings || [])].filter(Boolean),
    ),
  ].join(' ');
  const playTrack = async (trackId) => {
    if (!ready) return;
    try {
      await mutate('play_track', { trackId });
    } catch {
      /* Persistent error is rendered below. */
    }
  };
  const control = async (action, value = null) => {
    try {
      await mutate('control', { action, value });
    } catch {
      /* Reconcile state without replaying the action. */
    }
    await refresh();
  };
  const clearSearch = () => {
    setSearch('');
    setView({ query: '', page: 1 });
    inputRef.current?.focus();
  };
  const closeTrack = (success) => {
    const previous = activeTrack;
    setActiveTrack(null);
    requestAnimationFrame(() => {
      const action = previous.mode === 'remove' ? 'remove' : 'edit';
      const buttons = [...trackButtons.current.entries()]
        .filter(([key]) => key.endsWith(`:${action}`))
        .map(([, button]) => button);
      const target =
        success && action === 'remove'
          ? buttons[Math.min(previous.index, buttons.length - 1)]
          : trackButtons.current.get(`${previous.track.id}:${action}`);
      (target || libraryTitle.current)?.focus({ preventScroll: true });
    });
  };
  const submitDevice = async (event) => {
    event.preventDefault();
    if (busy || devicePending.current || deviceComposing.current || event.nativeEvent?.isComposing)
      return;
    const fail = (message) => {
      setDeviceError(message);
      urlRef.current?.focus();
    };
    if (!usable) {
      fail('客户端尚未就绪，请重新检查后再添加。');
      return;
    }
    try {
      const parsed = new URL(url.trim());
      if (!['http:', 'https:'].includes(parsed.protocol) || !parsed.hostname)
        throw Error('invalid');
    } catch {
      fail('请输入完整的 http 或 https 设备描述地址。');
      return;
    }
    devicePending.current = true;
    setDeviceError('');
    try {
      const result = await mutate('add_device', { url: url.trim() }, '已添加音响');
      if (result) {
        setPanel(null);
        setUrl('');
      }
    } catch (error) {
      fail(messageOf(error));
    } finally {
      devicePending.current = false;
    }
  };
  const emptyTitle = !available
    ? '请在桌面客户端中打开'
    : !loaded
      ? connectionError
        ? '暂时无法读取音乐库'
        : '正在读取音乐库'
      : query
        ? '没有找到这首歌'
        : '这里，留给你的好音乐';
  const emptyCopy = !available
    ? '此界面需要小米音响桌面客户端，无法在普通浏览器中控制音响。'
    : !loaded
      ? connectionError
        ? '请点击上方「重新检查」。如果仍未恢复，请重新启动客户端。'
        : '客户端正在准备本地音乐和播放设备。'
      : query
        ? `试试其他歌名或文件名，或清空「${query}」。`
        : '添加本地音乐，或导入整个文件夹。连接音响后，就能把音乐送到耳边。';
  return (
    <>
      <a className="skip-link" href="#library-title">
        跳到音乐库
      </a>
      <header className="topbar">
        <div className="brand" aria-label="小米音响本地音乐桌面客户端">
          <span className="brand-mark">
            <Icon name="music" />
          </span>
          <span>
            小米音响<span className="brand-divider">/</span>
            <span className="brand-sub">本地音乐</span>
          </span>
        </div>
        <span className={`connection ${connected ? 'online' : 'offline'}`} id="connection-status">
          <span className="status-dot" />
          {!available
            ? '需要桌面客户端'
            : connected
              ? '客户端已就绪'
              : connecting
                ? '正在启动'
                : '客户端未就绪'}
        </span>
      </header>
      <main className="workspace">
        <aside className="device-panel" aria-labelledby="device-title">
          <div className="section-heading">
            <span className="eyebrow">播放设备</span>
            <Icon name="speaker" className="muted" />
          </div>
          <div
            className={`speaker-art${device && state.device_connection === 'connected' ? ' connected' : ''}`}
            aria-hidden="true"
          >
            <div className="speaker-top">
              <span />
              <span />
              <span />
            </div>
            <div className="speaker-body">
              <div className="speaker-light" />
              <div className="speaker-grille" />
            </div>
            <div className="speaker-shadow" />
          </div>
          <div className="device-summary">
            <h1 id="device-title">{device?.name || '连接你的音响'}</h1>
            <p id="device-subtitle">
              {device
                ? [
                    state.device_connection === 'disconnected'
                      ? '已断开 · 自动重连中'
                      : state.device_connection === 'checking'
                        ? '正在连接'
                        : '已连接',
                    device.model,
                    device.address,
                  ]
                    .filter(Boolean)
                    .join(' · ')
                : '让喜欢的音乐，回到房间里。'}
            </p>
          </div>
          <button
            className="button primary discover-button"
            id="discover"
            disabled={blocked || discovering}
            aria-busy={discovering}
            onClick={discover}
          >
            <Icon name="refresh" />
            <span>
              {discovering
                ? '正在搜索音响'
                : state.device_connection === 'connected'
                  ? '搜索其他音响'
                  : '重新搜索音响'}
            </span>
          </button>
          <p className="device-hint" id="device-hint" role="status">
            {deviceHint}
          </p>
          <p
            className="discovery-error"
            id="discovery-error"
            role="status"
            hidden={!state.discovery_error}
          >
            {state.discovery_error}
          </p>
          <div className="device-list" id="devices" aria-label="已发现的音响">
            {devices.map((item) => {
              const selected = item.id === state.selected_device_id;
              const status =
                state.device_connection === 'disconnected'
                  ? '连接已断开，点击重试'
                  : state.device_connection === 'checking'
                    ? '正在连接'
                    : '已连接';
              return (
                <button
                  key={item.id}
                  className="device-option"
                  type="button"
                  data-device-id={item.id}
                  aria-pressed={selected}
                  aria-label={`${selected ? status : '选择'} ${item.name}${item.is_xiaomi_pro ? '，小爱音箱 Pro' : ''}`}
                  disabled={blocked}
                  onClick={async () => {
                    if (selected && state.device_connection !== 'disconnected') return;
                    try {
                      await mutate('select_device', { deviceId: item.id }, `正在连接 ${item.name}`);
                    } catch {
                      /* Persistent recovery error. */
                    }
                  }}
                >
                  <Icon name="speaker" />
                  <span className="device-label">
                    <span className="device-name-line">
                      <strong>{item.name}</strong>
                      {item.is_xiaomi_pro ? (
                        <span className="device-badge">小爱 Pro</span>
                      ) : item.is_xiaomi ? (
                        <span className="device-badge">小米</span>
                      ) : null}
                    </span>
                    <small>{selected ? status : item.model || item.address || 'DLNA 音响'}</small>
                  </span>
                  {selected && state.device_connection === 'connected' && (
                    <Icon name="check" className="small check" />
                  )}
                </button>
              );
            })}
          </div>
          <button
            className="text-button manual-button"
            id="open-device"
            disabled={blocked}
            onClick={() => {
              setDeviceError('');
              setPanel('device');
            }}
          >
            通过设备地址添加
            <Icon name="arrow" className="small" />
          </button>
          <div className="setup-note">
            <Icon name="info" />
            <div>
              <strong>连接之前</strong>
              <p>自动搜索小米 / 小爱音箱 Pro。电脑与音响需在同一局域网，音响需支持并开启 DLNA。</p>
            </div>
          </div>
          <button
            className="button outline voice-open"
            id="open-voice"
            disabled={blocked}
            onClick={() => setPanel('voice')}
          >
            小爱语音<span id="voice-badge">{voiceBadge}</span>
          </button>
          <div className="connection-route">
            <span className="route-computer">本机音乐</span>
            <span className="route-line" />
            <span>无线音响</span>
          </div>
        </aside>
        <section className="library-panel" aria-labelledby="library-title">
          <div className="library-heading">
            <div>
              <span className="eyebrow">你的私人曲库</span>
              <h2 ref={libraryTitle} id="library-title" tabIndex="-1">
                音乐库
                <span id="track-count" className="count">
                  {state.tracks.length.toLocaleString('zh-CN')}
                </span>
              </h2>
              <p>收藏在电脑里的好声音，即选即播。</p>
            </div>
          </div>
          <div className="notice" id="connection-error" role="status" hidden={connected}>
            <Icon name="info" />
            <p id="connection-error-text">{connectionError || '正在准备客户端，请稍候。'}</p>
            {available && (
              <button
                className="text-button"
                id="retry-connection"
                disabled={connecting}
                onClick={connect}
              >
                重新检查
              </button>
            )}
          </div>
          <div className="notice warning" id="playback-error" role="status" hidden={!errors}>
            <Icon name="info" />
            <p id="playback-error-text">{errors}</p>
          </div>
          <div className="library-toolbar">
            <div className="search-field">
              <Icon name="search" />
              <label className="sr-only" htmlFor="search">
                搜索音乐
              </label>
              <input
                ref={inputRef}
                id="search"
                type="search"
                placeholder="搜索歌曲或文件名"
                autoComplete="off"
                value={search}
                onCompositionStart={() => {
                  composing.current = true;
                }}
                onCompositionEnd={(event) => {
                  composing.current = false;
                  setView({ query: event.currentTarget.value, page: 1 });
                }}
                onChange={(event) => {
                  setSearch(event.target.value);
                  if (!composing.current && !event.nativeEvent.isComposing)
                    setView({ query: event.target.value, page: 1 });
                }}
              />
              {search && (
                <button
                  id="clear-search"
                  className="icon-button"
                  aria-label="清空搜索"
                  title="清空搜索"
                  onClick={clearSearch}
                >
                  <Icon name="close" className="small" />
                </button>
              )}
            </div>
            <button
              id="play-all"
              className="button outline"
              type="button"
              disabled={!ready || !state.tracks.length}
              onClick={() => control('play-all')}
            >
              播放全部
            </button>
            <span className="library-caption" id="library-caption">
              {state.tracks.length && reason ? reason : '本地音乐 · 直接推流'}
            </span>
          </div>
          <div className="track-surface">
            <div className="track-header" aria-hidden="true">
              <span>歌曲</span>
              <span>格式</span>
              <span>大小</span>
              <span />
            </div>
            <ul className="track-list" id="tracks" aria-label="音乐库中的歌曲">
              {tracks.map((track, index) => (
                <li
                  key={track.id}
                  className={`track-row${track.id === state.current_track_id ? ' current' : ''}`}
                  aria-current={track.id === state.current_track_id ? 'true' : undefined}
                >
                  <button
                    className="icon-button track-play"
                    type="button"
                    aria-label={`播放 ${track.title || track.filename}`}
                    title={reason || `播放 ${track.title || track.filename}`}
                    disabled={!ready}
                    onClick={() => playTrack(track.id)}
                  >
                    <Icon name="play" />
                  </button>
                  <div className="track-main">
                    <span className="track-number">
                      {track.id === state.current_track_id
                        ? '♪'
                        : String((page - 1) * 20 + index + 1).padStart(2, '0')}
                    </span>
                    <div className="track-text">
                      <strong className="track-title">{track.title || track.filename}</strong>
                      <span className="track-filename">{track.filename}</span>
                    </div>
                  </div>
                  <span className="track-format">{(track.format || '').toUpperCase()}</span>
                  <span className="track-size">{formatSize(track.size)}</span>
                  <div className="track-actions">
                    {['edit', 'remove'].map((action) => (
                      <button
                        ref={(button) => {
                          const key = `${track.id}:${action}`;
                          if (button) trackButtons.current.set(key, button);
                          else trackButtons.current.delete(key);
                        }}
                        key={action}
                        className={`text-button track-${action}`}
                        type="button"
                        data-track-id={track.id}
                        aria-label={`${action === 'edit' ? '识别或修改曲名' : '移出曲库'}：${track.title || track.filename}`}
                        title={action === 'edit' ? '识别或修改曲名' : '移出曲库，保留音频文件'}
                        disabled={blocked}
                        onClick={() => setActiveTrack({ mode: action, track: { ...track }, index })}
                      >
                        {action === 'edit' ? '曲名' : '移除'}
                      </button>
                    ))}
                  </div>
                </li>
              ))}
            </ul>
            {!tracks.length && (
              <div className="empty-state" id="empty-state">
                <div className="empty-record" aria-hidden="true">
                  <Icon name="music" />
                </div>
                <h3 id="empty-title">{emptyTitle}</h3>
                <p id="empty-copy">{emptyCopy}</p>
                <button
                  className="button outline"
                  id="empty-add"
                  disabled={!query && blocked}
                  onClick={() => (query ? clearSearch() : importMusic(false))}
                >
                  <Icon name="plus" />
                  <span>{query ? '清空搜索' : '添加第一首音乐'}</span>
                </button>
                {!query && loaded && (
                  <p className="format-hint" id="formats">
                    {(config?.formats || []).map((format) => format.toUpperCase()).join(' · ')}
                    {config?.max_tracks
                      ? ` · 最多 ${config.max_tracks.toLocaleString('zh-CN')} 首`
                      : ''}
                  </p>
                )}
              </div>
            )}
          </div>
          <p id="queue-help" className="field-help" role="status">
            {modeName} ·{' '}
            {current
              ? next
                ? `下一首：${next.title}`
                : '已到曲库最后一首，播完停止'
              : '点击任意歌曲后，按完整曲库顺序继续'}
            ；电脑应用需保持运行。
          </p>
          <div className="library-footer">
            <span id="list-status" role="status">
              {!loaded
                ? !available
                  ? '音乐库需要桌面客户端'
                  : connectionError
                    ? '音乐库暂时不可用'
                    : '正在读取音乐库…'
                : filtered.length
                  ? `第 ${((page - 1) * 20 + 1).toLocaleString('zh-CN')}–${Math.min(page * 20, filtered.length).toLocaleString('zh-CN')} 首，共 ${filtered.length.toLocaleString('zh-CN')} 首`
                  : query
                    ? '没有匹配的音乐'
                    : '还没有添加音乐'}
            </span>
            <nav className="pagination" aria-label="音乐列表分页">
              <button
                className="icon-button"
                id="page-previous"
                aria-label="上一页"
                title="上一页"
                disabled={!loaded || page <= 1}
                onClick={() => setView({ query, page: page - 1 })}
              >
                <Icon name="arrow" className="flip" />
              </button>
              <span id="page-info">
                {page} / {pages}
              </span>
              <button
                className="icon-button"
                id="page-next"
                aria-label="下一页"
                title="下一页"
                disabled={!loaded || page >= pages}
                onClick={() => setView({ query, page: page + 1 })}
              >
                <Icon name="arrow" />
              </button>
            </nav>
          </div>
          <section className="import-panel" aria-labelledby="import-title">
            <div className="source-toolbar">
              <div>
                <h3 id="import-title">导入音乐</h3>
                <span>添加本地文件，或将视频音轨保存到曲库。</span>
              </div>
              <div className="import-actions">
                <button
                  className="button outline"
                  id="open-folder"
                  disabled={blocked}
                  onClick={() => importMusic(true)}
                >
                  <Icon name="folder" />
                  <span>导入文件夹</span>
                </button>
                <button
                  className="button dark"
                  id="add-music"
                  disabled={blocked}
                  onClick={() => importMusic(false)}
                >
                  <Icon name="plus" />
                  <span>添加音乐</span>
                </button>
                <button
                  className="button outline"
                  id="open-mobile"
                  disabled={blocked}
                  onClick={() => setPanel('mobile')}
                >
                  手机点歌 / 导入
                </button>
                <button
                  className="button outline"
                  id="open-bilibili"
                  disabled={blocked}
                  onClick={() => setPanel('bilibili')}
                >
                  从 Bilibili 导入
                </button>
              </div>
            </div>
            {importStatus && (
              <div
                className={`import-status${importPending ? ' is-pending' : ''}`}
                id="import-status"
                role="status"
              >
                <Icon name="folder" />
                <div>
                  <strong id="import-status-title">{importStatus.title}</strong>
                  <p id="import-status-copy">{importStatus.copy}</p>
                </div>
              </div>
            )}
            <div ref={setQueueTarget} />
          </section>
          <p className="local-note">
            <span className="status-dot" />
            音乐直接从电脑传送，不上传云端。播放时请保持客户端运行。
          </p>
        </section>
      </main>
      <Player
        state={state}
        ready={ready}
        available={usable}
        busy={busy || importPending}
        reason={reason}
        control={control}
        playTrack={playTrack}
      />
      <div
        className="toast"
        id="toast"
        role="status"
        aria-live="polite"
        hidden={!toast}
        onMouseEnter={ui.pauseToast}
        onMouseLeave={ui.resumeToast}
      >
        {toast}
      </div>
      <Dialog
        id="device-dialog"
        title="添加音响地址"
        eyebrow="手动连接"
        description="manual-description"
        open={panel === 'device'}
        busy={busy}
        onClose={() => {
          if (!busy) setPanel(null);
        }}
      >
        <form
          id="device-form"
          className="dialog-body"
          noValidate
          onSubmit={submitDevice}
          onCompositionStart={() => {
            deviceComposing.current = true;
          }}
          onCompositionEnd={() => {
            deviceComposing.current = false;
          }}
        >
          <p id="manual-description" className="dialog-description">
            自动搜索未发现音响时，可填写它的 DLNA 设备描述地址。通常不需要手动设置。
          </p>
          <label className="field-label" htmlFor="device-url">
            设备描述地址
          </label>
          <input
            ref={urlRef}
            autoFocus
            data-autofocus
            className="text-input"
            id="device-url"
            type="url"
            placeholder="http://192.168.1.100:端口/description.xml"
            autoComplete="off"
            required
            disabled={busy}
            value={url}
            onChange={(event) => {
              setUrl(event.target.value);
              setDeviceError('');
            }}
            aria-invalid={Boolean(deviceError)}
            aria-describedby="device-url-help device-error"
          />
          <p className="field-help" id="device-url-help">
            需要完整的 http 或 https 地址，通常以 .xml 结尾。
          </p>
          <p className="field-error" id="device-error" role="alert">
            {deviceError}
          </p>
          <div className="dialog-actions">
            <button
              type="button"
              className="button outline"
              disabled={busy}
              onClick={() => setPanel(null)}
            >
              取消
            </button>
            <button
              type="submit"
              className="button primary"
              id="device-submit"
              disabled={busy}
              aria-busy={busy}
            >
              {busy ? '正在添加…' : '添加音响'}
            </button>
          </div>
        </form>
      </Dialog>
      <TrackDialogs
        active={activeTrack}
        onClose={closeTrack}
        mutate={mutate}
        invoke={ui.invoke}
        announce={announce}
      />
      <FeaturePanels
        panel={panel}
        onClose={() => setPanel(null)}
        invoke={ui.invoke}
        connected={connected}
        refresh={refresh}
        announce={announce}
        onVoiceBadge={setVoiceBadge}
        queueTarget={queueTarget}
      />
    </>
  );
}
