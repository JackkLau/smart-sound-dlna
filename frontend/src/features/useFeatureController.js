import { useCallback, useEffect, useRef, useState } from 'react';
import { messageOf } from '../utils.js';

export const emptyFeatures = {
  voice: { connected: false, listening: false, devices: [] },
  downloads: { authenticated: false, directory: '', jobs: [] },
};

export default function useFeatureController({
  invoke,
  connected,
  refresh,
  announce,
  onVoiceBadge,
}) {
  const [features, setFeatures] = useState(emptyFeatures);
  const [pending, setPending] = useState('');
  const [errors, setErrors] = useState({});
  const [focusTarget, setFocusTarget] = useState(null);
  useEffect(() => {
    if (!pending && focusTarget) {
      focusTarget.current?.focus();
      setFocusTarget(null);
    }
  }, [pending, focusTarget]);
  const current = useRef({
    invoke,
    connected,
    refresh,
    announce,
    onVoiceBadge,
  });
  current.current = { invoke, connected, refresh, announce, onVoiceBadge };
  const mounted = useRef(false);
  const polling = useRef(false);
  const generation = useRef(0);
  const busy = useRef(false);
  const previousPhases = useRef({});
  const completed = useRef(new Set());
  const poll = useCallback(async () => {
    if (polling.current || !current.current.connected || !current.current.invoke) return;
    polling.current = true;
    const revision = generation.current;
    try {
      const result = await current.current.invoke('get_features');
      if (!mounted.current || revision !== generation.current) return;
      setFeatures(result);
      setErrors((previous) => ({ ...previous, poll: '' }));
      for (const provider of ['xiaomi', 'bilibili']) {
        const phase = result[`${provider}_login`]?.phase || 'idle';
        if (previousPhases.current[provider] !== phase) {
          setErrors((previous) => ({ ...previous, [`${provider}-login`]: '' }));
          previousPhases.current[provider] = phase;
          if (phase === 'connected')
            current.current.announce(
              provider === 'xiaomi' ? '已连接小米账号，请选择接收语音的音响' : 'Bilibili 登录成功',
            );
        }
      }
      for (const job of result.downloads?.jobs || []) {
        if (job.phase === 'completed' && !completed.current.has(job.id)) {
          completed.current.add(job.id);
          current.current.announce(`已加入曲库：${job.title}`);
          current.current.refresh();
        }
      }
      current.current.onVoiceBadge?.(
        result.voice?.listening ? '接收中' : result.voice?.connected ? '已连接' : '未连接',
      );
    } catch (error) {
      if (mounted.current && revision === generation.current)
        setErrors((previous) => ({ ...previous, poll: messageOf(error) }));
    } finally {
      polling.current = false;
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    poll();
    const timer = setInterval(poll, 2000);
    return () => {
      mounted.current = false;
      generation.current += 1;
      clearInterval(timer);
    };
  }, [connected, invoke, poll]);
  const clearError = (key) => setErrors((previous) => ({ ...previous, [key]: '' }));
  const run = async (key, operation, field) => {
    if (busy.current || !current.current.connected) return;
    busy.current = true;
    generation.current += 1;
    setPending(key);
    clearError(key);
    try {
      return await operation();
    } catch (error) {
      if (mounted.current) {
        setErrors((previous) => ({ ...previous, [key]: messageOf(error) }));
        if (field) setFocusTarget(field);
      }
    } finally {
      busy.current = false;
      generation.current += 1;
      if (mounted.current) {
        setPending('');
        await poll();
      }
    }
  };
  return {
    features,
    pending,
    errors,
    run,
    poll,
    clearError,
    disabled: !connected || Boolean(pending),
    invoke,
    refresh,
    announce,
  };
}
