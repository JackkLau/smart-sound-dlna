import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Dialog, Icon } from '../../../frontend/src/components.jsx';
import { formatSize, formatTime } from '../../../frontend/src/utils.js';
import { request, uploadFile } from './api.js';

const EMPTY = {
  tracks: [],
  devices: [],
  selected_device_id: null,
  current_track_id: null,
  transport_state: 'STOPPED',
  position: 0,
  duration: 0,
  volume: null,
  queue_mode: 'sequential',
  warnings: [],
};
const PAGE_SIZE = 20;
const number = new Intl.NumberFormat('zh-CN');
const uploadLabels = { waiting: '等待上传', success: '已加入音乐库', cancelled: '已取消' };

function RangeControl({ id, label, value, max, disabled, onCommit, onDraft }) {
  const [draft, setDraft] = useState(value);
  const [focused, setFocused] = useState(false);
  const latestValue = useRef(value);
  latestValue.current = value;
  const draftValue = useRef(value);
  const dirty = useRef(false);
  const pending = useRef(false);
  useEffect(() => {
    if (!focused && !pending.current) {
      draftValue.current = value;
      dirty.current = false;
      setDraft(value);
      onDraft?.(null);
    }
  }, [value, focused, onDraft]);
  const commit = async () => {
    if (!dirty.current || pending.current) return;
    const next = Number(draftValue.current);
    dirty.current = false;
    if (disabled || next === Number(latestValue.current)) {
      draftValue.current = latestValue.current;
      setDraft(latestValue.current);
      onDraft?.(null);
      return;
    }
    pending.current = true;
    try {
      await onCommit(next);
    } finally {
      pending.current = false;
      if (!dirty.current) {
        draftValue.current = latestValue.current;
        setDraft(latestValue.current);
        onDraft?.(null);
      }
    }
  };
  return (
    <>
      <label className="sr-only" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        type="range"
        min="0"
        max={max}
        value={draft}
        step="1"
        disabled={disabled}
        aria-valuetext={
          id === 'seek' ? `${formatTime(draft)}，共 ${formatTime(max)}` : String(draft)
        }
        onFocus={() => {
          setFocused(true);
          onDraft?.(Number(draftValue.current));
        }}
        onChange={(event) => {
          dirty.current = true;
          draftValue.current = Number(event.target.value);
          setDraft(draftValue.current);
          onDraft?.(draftValue.current);
        }}
        onPointerUp={() => {
          void commit();
        }}
        onKeyUp={(event) => {
          if (
            !event.nativeEvent.isComposing &&
            [
              'ArrowLeft',
              'ArrowRight',
              'ArrowUp',
              'ArrowDown',
              'Home',
              'End',
              'PageUp',
              'PageDown',
            ].includes(event.key)
          )
            void commit();
        }}
        onBlur={() => {
          setFocused(false);
          void commit();
        }}
      />
    </>
  );
}

export default function LegacyApp({ fetcher = globalThis.fetch, makeXHR, pollInterval = 2000 }) {
  const [config, setConfig] = useState(null);
  const [state, setState] = useState(EMPTY);
  const [loaded, setLoaded] = useState(false);
  const [online, setOnline] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [connectionError, setConnectionError] = useState('');
  const [actionError, setActionError] = useState('');
  const [toast, setToast] = useState('');
  const [discoveryHint, setDiscoveryHint] = useState('音响需支持并开启 DLNA');
  const [discovering, setDiscovering] = useState(false);
  const [query, setQuery] = useState(() => new URLSearchParams(location.search).get('q') || '');
  const [searchDraft, setSearchDraft] = useState(query);
  const [page, setPage] = useState(() =>
    Math.max(1, Number.parseInt(new URLSearchParams(location.search).get('page') || '1', 10) || 1),
  );
  const [panel, setPanel] = useState(null);
  const [folderPath, setFolderPath] = useState('');
  const [deviceURL, setDeviceURL] = useState('');
  const [fieldError, setFieldError] = useState('');
  const [uploads, setUploads] = useState([]);
  const [uploadVisible, setUploadVisible] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [positionDraft, setPositionDraft] = useState(null);
  const [volumeDraft, setVolumeDraft] = useState(null);
  const latest = useRef({ config: null, online: false, state: EMPTY });
  latest.current = { config, online, state };
  const mounted = useRef(true);
  const lock = useRef(false);
  const connectingLock = useRef(false);
  const polling = useRef(false);
  const generation = useRef(0);
  const pollController = useRef(null);
  const composing = useRef(false);
  const formComposing = useRef(false);
  const searchInput = useRef(null);
  const dialogField = useRef(null);
  const fileInput = useRef(null);
  const toastTimer = useRef(null);
  const uploadItems = useRef([]);
  const uploadRunning = useRef(false);
  const uploadSerial = useRef(0);
  const activeUpload = useRef(null);
  const dragDepth = useRef(0);
  useEffect(() => {
    if (panel && !busy) dialogField.current?.focus();
  }, [panel, busy, fieldError]);

  const announce = useCallback((message) => {
    clearTimeout(toastTimer.current);
    setToast(message);
    toastTimer.current = setTimeout(() => {
      if (mounted.current) setToast('');
    }, 5500);
  }, []);
  const loadConfig = useCallback(async () => {
    const data = await request('/api/config', {}, 15000, fetcher);
    if (mounted.current) {
      latest.current.config = data;
      setConfig(data);
    }
    return data;
  }, [fetcher]);
  const acceptState = useCallback((data) => {
    if (!mounted.current) return;
    setState((previous) => ({
      ...previous,
      ...data,
      tracks: data.tracks || [],
      devices: data.devices || [],
    }));
    setLoaded(true);
  }, []);
  const refresh = useCallback(async () => {
    if (polling.current || lock.current || !mounted.current) return;
    polling.current = true;
    const controller = new AbortController();
    pollController.current = controller;
    const ownGeneration = generation.current;
    const timer = setTimeout(() => controller.abort(), 12000);
    try {
      const data = await request('/api/state', { signal: controller.signal }, 12000, fetcher);
      if (mounted.current && ownGeneration === generation.current) {
        acceptState(data);
        latest.current.online = true;
        setOnline(true);
        setConnectionError('');
      }
    } catch (error) {
      if (
        mounted.current &&
        ownGeneration === generation.current &&
        (!controller.signal.aborted || !lock.current)
      ) {
        latest.current.online = false;
        setOnline(false);
        setConnectionError(
          error.name === 'AbortError'
            ? '后台响应超时。将自动尝试重新连接，也可点击重新连接。'
            : error.message,
        );
      }
    } finally {
      clearTimeout(timer);
      if (pollController.current === controller) pollController.current = null;
      polling.current = false;
    }
  }, [acceptState, fetcher]);
  const connect = useCallback(async () => {
    if (connectingLock.current) return;
    connectingLock.current = true;
    setConnecting(true);
    try {
      await loadConfig();
      await refresh();
    } catch (error) {
      if (mounted.current) {
        latest.current.online = false;
        setOnline(false);
        setConnectionError(error.message);
      }
    } finally {
      connectingLock.current = false;
      if (mounted.current) setConnecting(false);
    }
  }, [loadConfig, refresh]);
  const mutate = useCallback(
    async (path, body, success = '') => {
      if (lock.current || !latest.current.config || !latest.current.online) return null;
      lock.current = true;
      generation.current += 1;
      pollController.current?.abort();
      setBusy(true);
      setActionError('');
      try {
        const data = await request(
          path,
          {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
              'X-Control-Token': latest.current.config.control_token,
            },
            body: JSON.stringify(body),
          },
          /discover|scan/.test(path) ? 60000 : 20000,
          fetcher,
        );
        if (Array.isArray(data.tracks)) acceptState(data);
        if (success && mounted.current) announce(success);
        return data;
      } catch (error) {
        if (error.status === 403) {
          try {
            await loadConfig();
          } catch {
            /* Never replay a failed side effect after token refresh. */
          }
        }
        if (mounted.current) setActionError(error.message);
        throw error;
      } finally {
        lock.current = false;
        if (mounted.current) setBusy(false);
      }
    },
    [acceptState, announce, fetcher, loadConfig],
  );

  useEffect(() => {
    mounted.current = true;
    connect();
    const interval = setInterval(() => {
      if (!latest.current.config && !polling.current && !lock.current) connect();
      else refresh();
    }, pollInterval);
    const beforeUnload = (event) => {
      if (uploadRunning.current) {
        event.preventDefault();
        event.returnValue = '';
      }
    };
    window.addEventListener('beforeunload', beforeUnload);
    return () => {
      mounted.current = false;
      clearInterval(interval);
      clearTimeout(toastTimer.current);
      generation.current += 1;
      pollController.current?.abort();
      activeUpload.current?.xhr.abort();
      window.removeEventListener('beforeunload', beforeUnload);
    };
  }, [connect, refresh, pollInterval]);

  const normalized = query.toLocaleLowerCase('zh-CN').trim();
  const filtered = state.tracks.filter((track) =>
    `${track.title} ${track.filename}`.toLocaleLowerCase('zh-CN').includes(normalized),
  );
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(Math.max(1, page), pages);
  useEffect(() => {
    if (loaded && page !== currentPage) setPage(currentPage);
    const url = new URL(location.href);
    if (query) url.searchParams.set('q', query);
    else url.searchParams.delete('q');
    if (currentPage > 1) url.searchParams.set('page', String(currentPage));
    else url.searchParams.delete('page');
    history.replaceState(null, '', url);
  }, [query, currentPage, loaded, page]);
  const selectedDevice = state.devices.find((device) => device.id === state.selected_device_id);
  const currentTrack = state.tracks.find((track) => track.id === state.current_track_id);
  const available = Boolean(online && config);
  const ready = available && Boolean(state.selected_device_id) && !busy;
  const playing = state.transport_state === 'PLAYING';
  const transitioning = state.transport_state === 'TRANSITIONING';
  const errors = [
    ...new Set([actionError, state.status_error, ...(state.warnings || [])].filter(Boolean)),
  ];
  useEffect(() => {
    document.title = currentTrack
      ? `${playing ? '正在播放 · ' : ''}${currentTrack.title || currentTrack.filename} | 小米音响`
      : '小米音响 · 本地音乐';
  }, [currentTrack, playing]);
  async function playTrack(trackId) {
    try {
      await mutate('/api/play', { track_id: trackId });
    } catch {
      /* Persistent inline error. */
    }
  }
  async function control(action, value) {
    try {
      await mutate('/api/control', { action, ...(value === undefined ? {} : { value }) });
    } catch {
      /* Persistent inline error. */
    }
    await refresh();
  }
  function togglePlay() {
    if (playing) control('pause');
    else if (state.transport_state === 'PAUSED_PLAYBACK') control('resume');
    else if (state.tracks.length) playTrack(state.current_track_id || state.tracks[0].id);
  }
  async function discover() {
    if (lock.current) return;
    setDiscovering(true);
    setDiscoveryHint('正在搜索，请稍等片刻…');
    try {
      const data = await mutate('/api/discover', {});
      if (data && mounted.current) {
        setState((previous) => ({ ...previous, devices: data.devices || [] }));
        setDiscoveryHint(
          data.devices?.length
            ? `发现 ${data.devices.length} 台音响，点击即可连接`
            : '暂未发现音响，请检查网络或手动添加',
        );
        announce(
          data.devices?.length
            ? `发现 ${data.devices.length} 台音响`
            : '未发现音响，可通过设备地址添加',
        );
      }
    } catch {
      if (mounted.current) setDiscoveryHint('搜索未完成，请重试或通过地址添加');
    } finally {
      if (mounted.current) setDiscovering(false);
      await refresh();
    }
  }
  function clearSearch() {
    setQuery('');
    setSearchDraft('');
    setPage(1);
    searchInput.current?.focus();
  }
  function openDialog(nextPanel) {
    setFieldError('');
    setPanel(nextPanel);
  }
  function showFieldError(message) {
    setFieldError(message);
    dialogField.current?.focus();
  }
  async function submitDialog(event) {
    event.preventDefault();
    if (formComposing.current || lock.current) return;
    if (!available) {
      showFieldError('本地服务未连接。请先重新连接，再重试。');
      return;
    }
    const value = (panel === 'folder' ? folderPath : deviceURL).trim();
    if (panel === 'folder' && !value) {
      showFieldError('请填写电脑上的文件夹完整路径。');
      return;
    }
    if (panel === 'device') {
      try {
        const url = new URL(value);
        if (!['http:', 'https:'].includes(url.protocol) || !url.hostname) throw new Error();
      } catch {
        showFieldError('请输入完整的 http 或 https 设备描述地址。');
        return;
      }
    }
    setFieldError('');
    try {
      const result = await mutate(
        panel === 'folder' ? '/api/library/scan' : '/api/devices',
        panel === 'folder' ? { path: value } : { url: value },
      );
      if (result && mounted.current) {
        announce(
          panel === 'folder'
            ? `已加入 ${result.added} 首音乐${result.skipped ? `，跳过 ${result.skipped} 个文件` : ''}`
            : '已添加音响，请在设备列表中选择',
        );
        setPanel(null);
        await refresh();
      }
    } catch (error) {
      if (mounted.current) showFieldError(error.message);
    }
  }

  function publishUploads() {
    if (mounted.current) setUploads(uploadItems.current.map((item) => ({ ...item })));
  }
  async function runUploads() {
    if (uploadRunning.current) return;
    uploadRunning.current = true;
    try {
      let item;
      while (
        mounted.current &&
        (item = uploadItems.current.find((entry) => entry.status === 'waiting'))
      ) {
        item.status = 'uploading';
        item.progress = 0;
        publishUploads();
        try {
          await uploadFile(
            item.file,
            latest.current.config.control_token,
            (progress) => {
              item.progress = progress;
              publishUploads();
            },
            (xhr) => {
              activeUpload.current = { id: item.id, xhr };
            },
            makeXHR,
          );
          item.status = 'success';
        } catch (error) {
          item.status = error.name === 'AbortError' ? 'cancelled' : 'error';
          item.error = error.message;
          if (error.status === 403 && mounted.current) {
            try {
              await loadConfig();
            } catch {
              /* Explicit retry remains available. */
            }
          }
        } finally {
          activeUpload.current = null;
          publishUploads();
        }
        await refresh();
      }
    } finally {
      uploadRunning.current = false;
      publishUploads();
      await refresh();
    }
  }
  function addFiles(files) {
    const currentConfig = latest.current.config;
    if (!currentConfig || !latest.current.online) {
      announce('请先连接本地服务，再添加音乐');
      return;
    }
    const incoming = Array.from(files || []);
    if (!incoming.length) return;
    if (
      incoming.length +
        uploadItems.current.filter((item) => ['waiting', 'uploading'].includes(item.status))
          .length >
      100
    ) {
      announce('一次最多添加 100 个文件，也可以使用文件夹导入');
      return;
    }
    if (!uploadRunning.current)
      uploadItems.current = uploadItems.current.filter((item) =>
        ['error', 'cancelled'].includes(item.status),
      );
    for (const file of incoming) {
      const extension = file.name.split('.').pop().toLowerCase();
      const error = !currentConfig.formats.includes(extension)
        ? '不支持此格式，请选择支持的音乐文件'
        : !file.size
          ? '文件为空，请选择有效的音乐文件'
          : file.size > currentConfig.max_upload_bytes
            ? `超过 ${formatSize(currentConfig.max_upload_bytes)}，可改用文件夹导入`
            : '';
      uploadItems.current.push({
        id: ++uploadSerial.current,
        file,
        status: error ? 'invalid' : 'waiting',
        progress: 0,
        error,
      });
    }
    setUploadVisible(true);
    publishUploads();
    runUploads();
  }
  function uploadAction(itemId) {
    const item = uploadItems.current.find((entry) => entry.id === itemId);
    if (!item) return;
    if (['error', 'cancelled'].includes(item.status)) {
      item.status = 'waiting';
      item.error = '';
      publishUploads();
      runUploads();
    } else if (item.status === 'invalid') {
      uploadItems.current = uploadItems.current.filter((entry) => entry.id !== item.id);
      publishUploads();
    } else {
      item.status = 'cancelled';
      if (activeUpload.current?.id === item.id) activeUpload.current.xhr.abort();
      publishUploads();
    }
  }
  const pendingUploads = uploads.filter((item) =>
    ['waiting', 'uploading'].includes(item.status),
  ).length;
  function cancelUploads() {
    if (!pendingUploads) {
      setUploadVisible(false);
      return;
    }
    uploadItems.current.forEach((item) => {
      if (['waiting', 'uploading'].includes(item.status)) item.status = 'cancelled';
    });
    activeUpload.current?.xhr.abort();
    publishUploads();
    announce('已取消剩余上传，已经加入的音乐会保留');
  }
  const chooseFiles = () => {
    if (available && !busy) fileInput.current?.click();
  };
  const listStatus = !loaded
    ? online
      ? '正在读取音乐库…'
      : '音乐库尚未连接'
    : filtered.length
      ? `第 ${number.format((currentPage - 1) * PAGE_SIZE + 1)}–${number.format(Math.min(currentPage * PAGE_SIZE, filtered.length))} 首，共 ${number.format(filtered.length)} 首`
      : query
        ? '没有匹配的音乐'
        : '还没有添加音乐';
  const transportLabel =
    {
      PLAYING: '正在播放',
      PAUSED_PLAYBACK: '已暂停',
      STOPPED: '已停止',
      TRANSITIONING: '正在连接音响…',
    }[state.transport_state] || '准备播放';
  const dialogId = panel === 'folder' ? 'folder-dialog' : 'device-dialog';

  return (
    <>
      <a className="skip-link" href="#library-title">
        跳到音乐库
      </a>
      <header className="topbar">
        <a className="brand" href="/" aria-label="小米音响本地音乐首页">
          <span className="brand-mark">
            <Icon name="music" />
          </span>
          <span>
            小米音响<span className="brand-divider">/</span>
            <span className="brand-sub">本地音乐</span>
          </span>
        </a>
        <span className={`connection ${online ? 'online' : 'offline'}`} id="connection-status">
          <span className="status-dot" />
          {online ? '本地服务已连接' : connecting ? '正在连接' : '本地服务未连接'}
        </span>
      </header>
      <main className="workspace">
        <aside className="device-panel" aria-labelledby="device-title">
          <div className="section-heading">
            <span className="eyebrow">播放设备</span>
            <Icon name="speaker" className="muted" />
          </div>
          <div className={`speaker-art ${selectedDevice ? 'connected' : ''}`} aria-hidden="true">
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
            <h1 id="device-title">{selectedDevice?.name || '连接你的音响'}</h1>
            <p>
              {selectedDevice
                ? [selectedDevice.model, selectedDevice.address].filter(Boolean).join(' · ') ||
                  '已选择，随时可以播放'
                : '让喜欢的音乐，回到房间里。'}
            </p>
          </div>
          <button
            className="button primary discover-button"
            disabled={!available || busy}
            aria-busy={discovering}
            onClick={discover}
          >
            <Icon name="refresh" />
            <span>搜索局域网音响</span>
          </button>
          <p className="device-hint" role="status">
            {discoveryHint}
          </p>
          <div className="device-list" aria-label="已发现的音响">
            {state.devices.map((device) => (
              <button
                key={device.id}
                className="device-option"
                disabled={!available || busy}
                aria-pressed={device.id === state.selected_device_id}
                aria-label={`${device.id === state.selected_device_id ? '已选择' : '连接'} ${device.name}`}
                onClick={async () => {
                  if (device.id !== state.selected_device_id) {
                    try {
                      await mutate(
                        '/api/select',
                        { device_id: device.id },
                        `已连接 ${device.name}`,
                      );
                    } catch {
                      /* Inline error. */
                    }
                  }
                }}
              >
                <Icon name="speaker" />
                <span className="device-label">
                  <strong>{device.name}</strong>
                  <small>{device.model || device.address || 'DLNA 音响'}</small>
                </span>
                {device.id === state.selected_device_id && (
                  <Icon name="check" className="small check" />
                )}
              </button>
            ))}
          </div>
          <button
            className="text-button manual-button"
            disabled={!available || busy}
            onClick={() => openDialog('device')}
          >
            通过设备地址添加
            <Icon name="arrow" className="small" />
          </button>
          <div className="setup-note">
            <Icon name="info" />
            <div>
              <strong>连接之前</strong>
              <p>电脑和音响需要连接到同一局域网。若未找到设备，请检查音响的 DLNA 设置。</p>
            </div>
          </div>
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
              <h2 id="library-title" tabIndex="-1">
                音乐库<span className="count">{number.format(state.tracks.length)}</span>
              </h2>
              <p>收藏在电脑里的好声音，即选即播。</p>
            </div>
            <div className="import-actions">
              <button
                className="button outline"
                disabled={!available || busy}
                onClick={() => openDialog('folder')}
              >
                <Icon name="folder" />
                导入文件夹
              </button>
              <button className="button dark" disabled={!available || busy} onClick={chooseFiles}>
                <Icon name="plus" />
                添加音乐
              </button>
            </div>
          </div>
          <input
            ref={fileInput}
            id="file-input"
            type="file"
            multiple
            hidden
            aria-label="选择本地音乐文件"
            accept={config?.formats.map((format) => `.${format}`).join(',')}
            onChange={(event) => {
              addFiles(event.target.files);
              event.target.value = '';
            }}
          />
          {!online && (
            <div className="notice" role="status">
              <Icon name="info" />
              <p>{connectionError || '无法连接后台程序。请确认程序仍在运行，再重新连接。'}</p>
              <button className="text-button" disabled={connecting} onClick={connect}>
                重新连接
              </button>
            </div>
          )}
          {errors.length > 0 && (
            <div className="notice warning" role="status">
              <Icon name="info" />
              <p>{errors.join(' ')}</p>
            </div>
          )}
          <div className="library-toolbar">
            <div className="search-field">
              <Icon name="search" />
              <label className="sr-only" htmlFor="search">
                搜索音乐
              </label>
              <input
                ref={searchInput}
                id="search"
                type="search"
                placeholder="搜索歌曲或文件名"
                autoComplete="off"
                value={searchDraft}
                onCompositionStart={() => {
                  composing.current = true;
                }}
                onCompositionEnd={(event) => {
                  composing.current = false;
                  setQuery(event.currentTarget.value);
                  setPage(1);
                }}
                onChange={(event) => {
                  setSearchDraft(event.target.value);
                  if (!composing.current && !event.nativeEvent.isComposing) {
                    setQuery(event.target.value);
                    setPage(1);
                  }
                }}
              />
              {searchDraft && (
                <button
                  className="icon-button"
                  aria-label="清空搜索"
                  title="清空搜索"
                  onClick={clearSearch}
                >
                  <Icon name="close" className="small" />
                </button>
              )}
            </div>
            <span className="library-caption">
              {state.tracks.length && !state.selected_device_id
                ? '先连接音响，再播放音乐'
                : '本地音乐 · 直接推流'}
            </span>
          </div>
          {uploadVisible && (
            <section className="upload-panel" aria-labelledby="upload-heading">
              <div className="upload-heading">
                <h3 id="upload-heading">添加音乐</h3>
                <button className="text-button" onClick={cancelUploads}>
                  {pendingUploads ? '取消剩余上传' : '收起上传记录'}
                </button>
              </div>
              <ul className="upload-list">
                {uploads.map((item) => {
                  const action = ['error', 'cancelled'].includes(item.status)
                    ? '重试'
                    : item.status === 'invalid'
                      ? '移除'
                      : '取消';
                  return (
                    <li className="upload-item" key={item.id}>
                      <strong>{item.file.name}</strong>
                      <span
                        className={
                          ['error', 'invalid'].includes(item.status) ? 'upload-failure' : ''
                        }
                      >
                        {formatSize(item.file.size)} ·{' '}
                        {item.status === 'uploading'
                          ? `${item.progress}% · 正在上传`
                          : uploadLabels[item.status] || item.error}
                      </span>
                      {item.status !== 'success' && (
                        <button
                          className="text-button"
                          aria-label={`${action} ${item.file.name}`}
                          onClick={() => uploadAction(item.id)}
                        >
                          {action}
                        </button>
                      )}
                      {item.status === 'uploading' && (
                        <progress
                          max="100"
                          value={item.progress}
                          aria-label={`${item.file.name} 上传进度`}
                        />
                      )}
                    </li>
                  );
                })}
              </ul>
              <p className="field-help" role="status">
                已加入 {uploads.filter((item) => item.status === 'success').length} 首
                {pendingUploads ? `，还有 ${pendingUploads} 首` : ''}
                {uploads.some((item) => ['error', 'invalid'].includes(item.status))
                  ? `，${uploads.filter((item) => ['error', 'invalid'].includes(item.status)).length} 个文件需要处理`
                  : ''}
              </p>
            </section>
          )}
          <div
            className={`track-surface ${dragging ? 'dragging' : ''}`}
            onDragEnter={(event) => {
              if (!event.dataTransfer.types.includes('Files')) return;
              event.preventDefault();
              dragDepth.current += 1;
              setDragging(true);
            }}
            onDragOver={(event) => {
              if (event.dataTransfer.types.includes('Files')) event.preventDefault();
            }}
            onDragLeave={() => {
              dragDepth.current = Math.max(0, dragDepth.current - 1);
              if (!dragDepth.current) setDragging(false);
            }}
            onDrop={(event) => {
              event.preventDefault();
              dragDepth.current = 0;
              setDragging(false);
              addFiles(event.dataTransfer.files);
            }}
          >
            <div className="track-header" aria-hidden="true">
              <span>歌曲</span>
              <span>格式</span>
              <span>大小</span>
              <span />
            </div>
            <ul className="track-list" aria-label="音乐列表">
              {filtered
                .slice((currentPage - 1) * PAGE_SIZE, currentPage * PAGE_SIZE)
                .map((track, index) => (
                  <li
                    className={`track-row ${track.id === state.current_track_id ? 'current' : ''}`}
                    key={track.id}
                  >
                    <div className="track-main">
                      <span className="track-number">
                        {track.id === state.current_track_id
                          ? '♪'
                          : String((currentPage - 1) * PAGE_SIZE + index + 1).padStart(2, '0')}
                      </span>
                      <div className="track-text">
                        <strong className="track-title">{track.title || track.filename}</strong>
                        <span className="track-filename">{track.filename}</span>
                      </div>
                    </div>
                    <span className="track-format">{track.format?.toUpperCase()}</span>
                    <span className="track-size">{formatSize(track.size || 0)}</span>
                    <button
                      className="icon-button track-play"
                      disabled={!ready}
                      aria-label={`播放 ${track.title || track.filename}`}
                      title={
                        state.selected_device_id
                          ? `播放 ${track.title || track.filename}`
                          : '请先连接音响'
                      }
                      onClick={() => playTrack(track.id)}
                    >
                      <Icon name="play" />
                    </button>
                  </li>
                ))}
            </ul>
            {!filtered.length && (
              <div className="empty-state">
                <div className="empty-record" aria-hidden="true">
                  <Icon name="music" />
                </div>
                <h3>
                  {!loaded
                    ? online
                      ? '正在读取音乐库'
                      : '等待本地服务连接'
                    : query
                      ? '没有找到这首歌'
                      : '这里，留给你的好音乐'}
                </h3>
                <p>
                  {!loaded
                    ? '请保持后台程序运行，连接后将显示你的音乐。'
                    : query
                      ? `试试其他歌名或文件名，或清空「${query}」。`
                      : '添加本地音乐，或导入整个文件夹。连接音响后，就能把音乐送到耳边。'}
                </p>
                <button
                  className="button outline"
                  disabled={!query && (!available || busy)}
                  onClick={() => (query ? clearSearch() : chooseFiles())}
                >
                  <Icon name="plus" />
                  <span>{query ? '清空搜索' : '添加第一首音乐'}</span>
                </button>
                {!query && loaded && (
                  <p className="format-hint">
                    {config?.formats.map((format) => format.toUpperCase()).join(' · ')} · 每首最大{' '}
                    {formatSize(config?.max_upload_bytes || 0)}
                  </p>
                )}
              </div>
            )}
            <div className="drop-overlay" aria-hidden="true">
              <Icon name="upload" />
              <span>松开鼠标，添加音乐</span>
            </div>
          </div>
          <div className="library-footer">
            <span role="status">{listStatus}</span>
            <nav className="pagination" aria-label="音乐列表分页">
              <button
                className="icon-button"
                aria-label="上一页"
                title="上一页"
                disabled={currentPage <= 1}
                onClick={() => setPage(currentPage - 1)}
              >
                <Icon name="arrow" className="flip" />
              </button>
              <span>
                {currentPage} / {pages}
              </span>
              <button
                className="icon-button"
                aria-label="下一页"
                title="下一页"
                disabled={currentPage >= pages}
                onClick={() => setPage(currentPage + 1)}
              >
                <Icon name="arrow" />
              </button>
            </nav>
          </div>
          <p className="local-note">
            <span className="status-dot" />
            音乐直接从电脑传送。可关闭浏览器，播放时请保持后台程序运行。
          </p>
        </section>
      </main>
      <footer className={`player ${playing ? 'is-playing' : ''}`} aria-label="音乐播放控制">
        <div className="now-playing">
          <div className="album-icon">
            <Icon name="music" />
          </div>
          <div className="now-copy">
            <strong>
              {currentTrack ? currentTrack.title || currentTrack.filename : '还没有播放音乐'}
            </strong>
            <span>
              {currentTrack
                ? `${transportLabel}${selectedDevice ? ` · ${selectedDevice.name}` : ''}`
                : selectedDevice
                  ? `播放到 ${selectedDevice.name}`
                  : '连接音响，选一首喜欢的歌'}
            </span>
          </div>
        </div>
        <div className="transport">
          <div className="transport-buttons">
            <button
              className="icon-button"
              aria-label="上一首"
              title="上一首"
              disabled={!ready || !state.tracks.length}
              onClick={() => control('previous')}
            >
              <Icon name="previous" />
            </button>
            <button
              className="play-button"
              aria-label={
                playing ? '暂停' : state.transport_state === 'PAUSED_PLAYBACK' ? '继续播放' : '播放'
              }
              disabled={!ready || !state.tracks.length || transitioning}
              aria-busy={transitioning || busy}
              onClick={togglePlay}
            >
              <Icon name={playing ? 'pause' : 'play'} />
            </button>
            <button
              className="icon-button"
              aria-label="下一首"
              title="下一首"
              disabled={!ready || !state.tracks.length}
              onClick={() => control('next')}
            >
              <Icon name="next" />
            </button>
            <button
              className="icon-button stop-button"
              aria-label="停止"
              title="停止"
              disabled={!ready || !currentTrack || state.transport_state === 'STOPPED'}
              onClick={() => control('stop')}
            >
              <Icon name="stop" className="small" />
            </button>
          </div>
          <div className="timeline">
            <span>{formatTime(positionDraft ?? state.position)}</span>
            <RangeControl
              id="seek"
              label="播放进度"
              value={Math.min(state.position || 0, state.duration || 0)}
              max={Math.max(1, state.duration || 0)}
              disabled={
                !ready || !currentTrack || !state.duration || state.transport_state === 'STOPPED'
              }
              onDraft={setPositionDraft}
              onCommit={(value) => control('seek', value)}
            />
            <span>{formatTime(state.duration)}</span>
          </div>
        </div>
        <div className="player-settings">
          <label className="sr-only" htmlFor="mode">
            播放顺序
          </label>
          <select
            id="mode"
            aria-label="播放顺序"
            value={state.queue_mode || 'sequential'}
            disabled={!available || busy}
            onChange={(event) => control('mode', event.target.value)}
          >
            <option value="sequential">顺序播放</option>
            <option value="repeat-all">列表循环</option>
            <option value="repeat-one">单曲循环</option>
          </select>
          <div className="volume-control">
            <Icon name="volume" />
            <RangeControl
              id="volume"
              label="音量"
              value={state.volume ?? 30}
              max={100}
              disabled={!ready || state.volume === null}
              onDraft={setVolumeDraft}
              onCommit={(value) => control('volume', value)}
            />
            <output htmlFor="volume">{volumeDraft ?? state.volume ?? '—'}</output>
          </div>
        </div>
      </footer>
      <div
        className="toast"
        role="status"
        aria-live="polite"
        hidden={!toast}
        onMouseEnter={() => clearTimeout(toastTimer.current)}
        onMouseLeave={() => {
          toastTimer.current = setTimeout(() => setToast(''), 3500);
        }}
      >
        {toast}
      </div>
      <Dialog
        id={dialogId}
        open={Boolean(panel)}
        title={panel === 'folder' ? '导入音乐文件夹' : '添加音响地址'}
        eyebrow={panel === 'folder' ? '从电脑导入' : '手动连接'}
        description="legacy-dialog-description"
        busy={busy}
        onClose={() => setPanel(null)}
      >
        <form
          className="dialog-body"
          noValidate
          onSubmit={submitDialog}
          onCompositionStart={() => {
            formComposing.current = true;
          }}
          onCompositionEnd={() => {
            formComposing.current = false;
          }}
        >
          <p className="dialog-description" id="legacy-dialog-description">
            {panel === 'folder'
              ? '填写运行后台程序这台电脑上的文件夹路径。子文件夹中的音乐也会一并加入，不会复制原文件。'
              : '自动搜索未发现音响时，可填写它的 DLNA 设备描述地址。'}
          </p>
          <label className="field-label" htmlFor="legacy-dialog-field">
            {panel === 'folder' ? '文件夹完整路径' : '设备描述地址'}
          </label>
          <input
            ref={dialogField}
            autoFocus
            className="text-input"
            id="legacy-dialog-field"
            type={panel === 'folder' ? 'text' : 'url'}
            autoComplete="off"
            value={panel === 'folder' ? folderPath : deviceURL}
            onChange={(event) =>
              panel === 'folder'
                ? setFolderPath(event.target.value)
                : setDeviceURL(event.target.value)
            }
            disabled={busy}
            aria-invalid={Boolean(fieldError)}
            aria-describedby="legacy-field-help legacy-field-error"
            placeholder={
              panel === 'folder'
                ? '例如 /Users/你的名字/Music'
                : 'http://192.168.1.100:端口/description.xml'
            }
          />
          <p className="field-help" id="legacy-field-help">
            {panel === 'folder'
              ? '填写文件夹的绝对路径；也支持以 ~ 表示个人目录。无需添加引号。'
              : '需要完整的 http 或 https 地址，通常以 .xml 结尾。'}
          </p>
          <p className="field-error" id="legacy-field-error" role="alert">
            {fieldError}
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
            <button type="submit" className="button primary" disabled={busy} aria-busy={busy}>
              {panel === 'folder' ? '导入文件夹' : '添加音响'}
            </button>
          </div>
        </form>
      </Dialog>
    </>
  );
}
