import { useEffect, useRef, useState } from 'react';
import { Icon, Select } from './components.jsx';
import { formatTime } from './utils.js';

function Range({ id, value, max, disabled, onCommit, time = false }) {
  const [draft, setDraft] = useState(value ?? 30);
  const [focused, setFocused] = useState(false);
  const focusRef = useRef(false);
  const latestValue = useRef(value);
  latestValue.current = value;
  const committed = useRef(value);
  useEffect(() => {
    if (!focused) {
      setDraft(value ?? 30);
      committed.current = value;
    }
  }, [value, focused]);
  const commit = async (event) => {
    const next = Number(event.currentTarget.value);
    if (disabled || next === committed.current) return;
    committed.current = next;
    try {
      await onCommit(next);
    } finally {
      if (!focusRef.current) setDraft(latestValue.current ?? 30);
    }
  };
  return (
    <>
      {time && <span id="elapsed">{formatTime(draft)}</span>}
      <label className="sr-only" htmlFor={id}>
        {time ? '播放进度' : '音量'}
      </label>
      <input
        id={id}
        type="range"
        style={{
          '--range-fill': `${Math.min(100, Math.max(0, (draft / Math.max(1, max)) * 100))}%`,
        }}
        min="0"
        max={Math.max(1, max)}
        step="1"
        value={draft}
        disabled={disabled}
        onFocus={() => {
          focusRef.current = true;
          setFocused(true);
        }}
        onBlur={(event) => {
          focusRef.current = false;
          setFocused(false);
          void commit(event);
        }}
        onChange={(event) => setDraft(Number(event.target.value))}
        onPointerUp={commit}
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
            void commit(event);
        }}
        aria-valuetext={time ? `${formatTime(draft)}，共 ${formatTime(max)}` : String(draft)}
      />
      {time ? (
        <span id="duration">{formatTime(max)}</span>
      ) : (
        <output htmlFor={id} id="volume-label">
          {value === null ? '—' : draft}
        </output>
      )}
    </>
  );
}
export default function Player({ state, ready, available, busy, reason, control, playTrack }) {
  const current = state.tracks.find((track) => track.id === state.current_track_id);
  const device = state.devices.find((item) => item.id === state.selected_device_id);
  const playing = Boolean(current) && state.transport_state === 'PLAYING';
  const paused = Boolean(current) && state.transport_state === 'PAUSED_PLAYBACK';
  const external = !current && ['PLAYING', 'PAUSED_PLAYBACK'].includes(state.transport_state);
  const transitioning = state.transport_state === 'TRANSITIONING';
  const labels = {
    PLAYING: '正在播放',
    PAUSED_PLAYBACK: '已暂停',
    STOPPED: '已停止',
    NO_MEDIA_PRESENT: '已停止',
    TRANSITIONING: '正在准备播放…',
  };
  const title = current
    ? current.title || current.filename
    : external
      ? state.transport_state === 'PLAYING'
        ? '音响正在播放其他内容'
        : '音响已暂停其他内容'
      : '还没有播放音乐';
  const subtitle =
    state.selected_device_id && reason
      ? reason
      : current
        ? `${labels[state.transport_state] || '准备播放'}${device ? ` · ${device.name}` : ''}`
        : external
          ? '选择本地音乐，即可切换播放'
          : device
            ? `播放到 ${device.name}`
            : '连接音响，选一首喜欢的歌';
  useEffect(() => {
    document.title = current
      ? `${playing ? '正在播放 · ' : ''}${current.title || current.filename} | 小米音响`
      : '小米音响 · 本地音乐';
  }, [current, playing]);
  const toggle = () => {
    if (playing) control('pause');
    else if (paused) control('resume');
    else if (state.tracks.length) playTrack(current?.id || state.tracks[0].id);
  };
  const playLabel = playing ? '暂停' : paused ? '继续播放' : '播放';
  return (
    <footer className={`player${playing ? ' is-playing' : ''}`} aria-label="音乐播放控制">
      <div className="now-playing">
        <div className="album-icon">
          <Icon name="music" />
        </div>
        <div className="now-copy">
          <strong id="now-title">{title}</strong>
          <span id="now-subtitle">{subtitle}</span>
        </div>
      </div>
      <div className="transport">
        <div className="transport-buttons">
          <button
            className="icon-button"
            id="previous"
            aria-label="上一首"
            title="上一首"
            disabled={!ready || !state.tracks.length}
            onClick={() => control('previous')}
          >
            <Icon name="previous" />
          </button>
          <button
            className="play-button"
            id="play-pause"
            aria-label={playLabel}
            title={reason || playLabel}
            disabled={!ready || !state.tracks.length || transitioning}
            aria-busy={transitioning || busy}
            onClick={toggle}
          >
            <Icon name={playing ? 'pause' : 'play'} />
          </button>
          <button
            className="icon-button"
            id="next"
            aria-label="下一首"
            title="下一首"
            disabled={!ready || !state.tracks.length}
            onClick={() => control('next')}
          >
            <Icon name="next" />
          </button>
          <button
            className="icon-button stop-button"
            id="stop"
            aria-label="停止"
            title="停止"
            disabled={!ready || !current || state.transport_state === 'STOPPED'}
            onClick={() => control('stop')}
          >
            <Icon name="stop" className="small" />
          </button>
        </div>
        <div className="timeline">
          <Range
            id="seek"
            value={Math.min(state.position || 0, state.duration || 0)}
            max={state.duration || 0}
            disabled={!ready || !current || !state.duration || state.transport_state === 'STOPPED'}
            onCommit={(value) => control('seek', value)}
            time
          />
        </div>
      </div>
      <div className="player-settings">
        <label className="sr-only" htmlFor="mode">
          播放顺序
        </label>
        <Select
          id="mode"
          aria-label="播放顺序"
          value={state.queue_mode || 'sequential'}
          disabled={!available || busy}
          onChange={(event) => control('mode', event.target.value)}
        >
          <option value="sequential">顺序播放</option>
          <option value="repeat-all">列表循环</option>
          <option value="repeat-one">单曲循环</option>
        </Select>
        <div className="volume-control">
          <Icon name="volume" />
          <Range
            id="volume"
            value={state.volume}
            max={100}
            disabled={!ready || state.volume === null}
            onCommit={(value) => control('volume', value)}
          />
        </div>
      </div>
    </footer>
  );
}
