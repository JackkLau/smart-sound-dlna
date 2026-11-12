import { useCallback, useEffect, useRef, useState } from 'react';
import { messageOf } from '../utils.js';
import type { Invoke, Mutate, VideoSnapshot } from './types';

const initial: VideoSnapshot = {
  enabled: false,
  owner_kind: 'none',
  entry: null,
  probe: null,
  plan: null,
  session: null,
  owner_revision: 0,
  music_active: false,
  error: null,
};

export function videoMessage(value: unknown): string {
  return messageOf(value).replace(/^[A-Z][A-Z0-9_]+[:：]\s*/, '');
}

function snapshot(value: unknown): VideoSnapshot {
  if (
    !value ||
    typeof value !== 'object' ||
    !('enabled' in value) ||
    typeof value.enabled !== 'boolean' ||
    !('owner_revision' in value) ||
    !Number.isSafeInteger(value.owner_revision)
  )
    throw new Error('影视状态无法读取，请重新检查客户端。');
  return value as VideoSnapshot;
}

export function useVideo({
  enabled,
  invoke,
  mutate,
  busy,
  ownerRevision = 0,
  refreshMusic,
  getMutationState,
}: {
  enabled: boolean;
  invoke: Invoke;
  mutate: Mutate;
  busy: boolean;
  ownerRevision?: number;
  refreshMusic: () => Promise<unknown>;
  getMutationState: () => { generation: number; locked: boolean };
}) {
  const [state, setState] = useState<VideoSnapshot>(initial);
  const [error, setError] = useState('');
  const [readError, setReadError] = useState('');
  const [pending, setPending] = useState(false);
  const refs = useRef({
    mounted: true,
    generation: 0,
    polling: false,
    mutating: false,
    busy,
    ownerRevision,
  });
  refs.current.busy = busy;
  refs.current.ownerRevision = ownerRevision;

  const accept = useCallback((value: unknown) => {
    const next = snapshot(value);
    if (next.owner_revision < refs.current.ownerRevision) return;
    setState(next);
  }, []);
  const refresh = useCallback(async () => {
    const current = refs.current;
    const foreground = getMutationState();
    if (!enabled || current.polling || current.mutating || current.busy || foreground.locked)
      return;
    const generation = current.generation;
    current.polling = true;
    try {
      const value = await invoke('get_video_state');
      const latest = getMutationState();
      if (
        current.mounted &&
        generation === current.generation &&
        foreground.generation === latest.generation &&
        !latest.locked
      ) {
        accept(value);
        setReadError('');
      }
    } catch (failure) {
      const latest = getMutationState();
      if (
        current.mounted &&
        generation === current.generation &&
        foreground.generation === latest.generation &&
        !latest.locked
      )
        setReadError(videoMessage(failure));
    } finally {
      current.polling = false;
    }
  }, [enabled, invoke, accept, getMutationState]);

  const run = useCallback(
    async (command: string, args: Record<string, unknown> = {}) => {
      const current = refs.current;
      if (!enabled || current.mutating || current.busy) return null;
      current.mutating = true;
      current.generation += 1;
      setPending(true);
      setError('');
      try {
        // Reuse the desktop mutation owner. No second foreground command queue.
        const value = await mutate(command, args, '', { scopedError: true });
        if (value !== null && current.mounted) accept(value);
        return value === null ? null : snapshot(value);
      } catch (failure) {
        if (current.mounted) setError(videoMessage(failure));
        return null;
      } finally {
        current.mutating = false;
        if (current.mounted) {
          setPending(false);
          void refreshMusic();
        }
      }
    },
    [enabled, mutate, accept, refreshMusic],
  );

  useEffect(() => {
    const current = refs.current;
    if (busy && !current.mutating) current.generation += 1;
    if (!busy) void refresh();
  }, [busy, ownerRevision, refresh]);
  useEffect(() => {
    const current = refs.current;
    current.mounted = true;
    if (!enabled) {
      setState(initial);
      setError('');
      setReadError('');
      return;
    }
    void refresh();
    const timer = setInterval(() => void refresh(), 2000);
    return () => {
      current.mounted = false;
      current.generation += 1;
      clearInterval(timer);
    };
  }, [enabled, refresh]);

  return { state, error: error || readError, pending, refresh, run };
}

export type VideoController = ReturnType<typeof useVideo>;
