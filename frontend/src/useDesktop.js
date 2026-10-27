import { useCallback, useEffect, useRef, useState } from 'react';
import { messageOf } from './utils.js';

export const initialState = {
  tracks: [],
  devices: [],
  selected_device_id: null,
  device_connection: 'unselected',
  current_track_id: null,
  transport_state: 'STOPPED',
  position: 0,
  duration: 0,
  volume: null,
  status_error: null,
  warnings: [],
  queue_mode: 'sequential',
  discovering: false,
  discovery_error: null,
  interfaces: [],
  media_port: 0,
};
const desktopInvoke = (...args) => window.__TAURI__.core.invoke(...args);
const browserMessage =
  '请从已安装的桌面客户端打开。此界面需要客户端提供音乐文件访问和音响控制，无法单独在浏览器中运行。';

export function useDesktop(providedInvoke) {
  const available = Boolean(providedInvoke || window.__TAURI__?.core?.invoke);
  const invoke = providedInvoke || desktopInvoke;
  const [state, setState] = useState(initialState);
  const [config, setConfig] = useState(null);
  const [loaded, setLoaded] = useState(false);
  const [connected, setConnected] = useState(false);
  const [connectionError, setConnectionError] = useState(available ? '' : browserMessage);
  const [busy, setBusy] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [discoveryPending, setDiscoveryPending] = useState(false);
  const [importPending, setImportPending] = useState(false);
  const [importStatus, setImportStatus] = useState(null);
  const [actionError, setActionError] = useState('');
  const [slowMessage, setSlowMessage] = useState('');
  const [toast, setToast] = useState('');
  const refs = useRef({
    generation: 0,
    polling: false,
    locked: false,
    connecting: false,
    config: null,
    connected: false,
    state: initialState,
  });
  const toastTimer = useRef(null);
  const announce = useCallback((message) => {
    clearTimeout(toastTimer.current);
    setToast(message);
    toastTimer.current = setTimeout(() => setToast(''), 5500);
  }, []);
  const pauseToast = () => clearTimeout(toastTimer.current);
  const resumeToast = () => {
    toastTimer.current = setTimeout(() => setToast(''), 3500);
  };
  const acceptState = useCallback((data) => {
    const next = {
      ...refs.current.state,
      ...data,
      tracks: Array.isArray(data.tracks) ? data.tracks : [],
      devices: Array.isArray(data.devices) ? data.devices : [],
    };
    refs.current.state = next;
    setState(next);
    setLoaded(true);
  }, []);
  const connection = useCallback((value, error = '') => {
    refs.current.connected = value;
    setConnected(value);
    setConnectionError(error);
  }, []);
  const refresh = useCallback(async () => {
    const current = refs.current;
    if (!available || current.polling || current.locked) return;
    current.polling = true;
    const generation = current.generation;
    const timer = setTimeout(() => {
      if (generation === current.generation)
        connection(false, '客户端响应较慢，正在等待当前操作完成。');
    }, 12000);
    try {
      const data = await invoke('get_state');
      if (generation === current.generation) {
        acceptState(data);
        connection(true);
      }
    } catch (error) {
      if (generation === current.generation) connection(false, messageOf(error));
    } finally {
      clearTimeout(timer);
      current.polling = false;
    }
  }, [available, invoke, acceptState, connection]);
  const connect = useCallback(async () => {
    const current = refs.current;
    if (!available || current.connecting) return;
    current.connecting = true;
    setConnecting(true);
    try {
      const data = await invoke('get_config');
      current.config = data;
      setConfig(data);
      await refresh();
    } catch (error) {
      connection(false, messageOf(error));
    } finally {
      current.connecting = false;
      setConnecting(false);
    }
  }, [available, invoke, refresh, connection]);
  const mutate = useCallback(
    async (command, args = {}, successMessage = '', { scopedError = false } = {}) => {
      const current = refs.current;
      if (current.locked || !current.config || !current.connected) return null;
      current.locked = true;
      current.generation += 1;
      setBusy(true);
      setActionError('');
      setSlowMessage('');
      const timer = setTimeout(
        () => setSlowMessage('操作仍在进行，请稍候。为避免重复执行，完成前无法再次操作。'),
        15000,
      );
      try {
        const result = await invoke(command, args);
        if (result && Array.isArray(result.tracks)) acceptState(result);
        if (successMessage && result !== null) announce(successMessage);
        return result;
      } catch (error) {
        if (!scopedError) setActionError(messageOf(error));
        throw error;
      } finally {
        clearTimeout(timer);
        current.locked = false;
        setBusy(false);
        setSlowMessage('');
      }
    },
    [invoke, acceptState, announce],
  );
  const importMusic = useCallback(
    async (folder = false) => {
      const current = refs.current;
      if (current.locked || !current.config || !current.connected) return;
      current.locked = true;
      current.generation += 1;
      setImportPending(true);
      setActionError('');
      setImportStatus({
        title: folder ? '请选择要导入的文件夹' : '请选择音乐文件',
        copy: '将在系统选择对话框中确认；不会复制或上传音乐。',
      });
      const timer = setTimeout(
        () =>
          setImportStatus({
            title: '正在等待选择或导入',
            copy: '可在系统选择对话框中取消；大文件夹可能需要一些时间。',
          }),
        15000,
      );
      try {
        const result = await invoke(folder ? 'pick_music_folder' : 'pick_music_files');
        if (result === null) {
          setImportStatus(null);
          return;
        }
        const added = Number(result.added) || 0;
        const skipped = Number(result.skipped) || 0;
        setImportStatus({
          title: added ? `已加入 ${added.toLocaleString('zh-CN')} 首音乐` : '未添加新的音乐',
          copy: [
            `曲库共 ${Number(result.total || 0).toLocaleString('zh-CN')} 首`,
            skipped ? `跳过 ${skipped.toLocaleString('zh-CN')} 个文件` : '',
            ...(result.warnings || []),
          ]
            .filter(Boolean)
            .join('。'),
        });
        announce(
          added ? `已加入 ${added.toLocaleString('zh-CN')} 首音乐` : '已检查所选音乐，没有新增文件',
        );
      } catch (error) {
        setActionError(messageOf(error));
        setImportStatus({
          title: '导入未完成',
          copy: `${messageOf(error)} 可重新选择文件或文件夹再试。`,
        });
      } finally {
        clearTimeout(timer);
        current.locked = false;
        setImportPending(false);
        await refresh();
      }
    },
    [invoke, announce, refresh],
  );
  const discover = useCallback(async () => {
    if (refs.current.locked || refs.current.state.discovering) return;
    setDiscoveryPending(true);
    try {
      const result = await mutate('discover_devices');
      if (result && !result.discovering && !result.discovery_error)
        announce(
          result.devices.length
            ? `发现 ${result.devices.length} 台音响`
            : '暂未发现音响，请检查网络或手动添加',
        );
    } catch {
      /* The persistent action error supplies recovery information. */
    } finally {
      setDiscoveryPending(false);
      await refresh();
    }
  }, [mutate, announce, refresh]);
  useEffect(() => {
    if (!available) return;
    void connect();
    const timer = setInterval(() => {
      if (!refs.current.config) void connect();
      else void refresh();
    }, 2000);
    return () => {
      clearInterval(timer);
      clearTimeout(toastTimer.current);
      refs.current.generation += 1;
    };
  }, [available, connect, refresh]);
  return {
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
    pauseToast,
    resumeToast,
    refresh,
    connect,
    mutate,
    invoke,
    importMusic,
    discover,
  };
}
