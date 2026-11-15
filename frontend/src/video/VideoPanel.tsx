import { useEffect, useRef, useState } from 'react';
import { Dialog, Icon, Select } from '../components.jsx';
import { Range } from '../Player.jsx';
import { formatTime } from '../utils.js';
import SubtitleOptions from './SubtitleOptions';
import MagnetDownload from './MagnetDownload';
import type { Invoke, SubtitleSelection, VideoDevice, VideoInfo, VideoSession } from './types';
import { videoMessage, type VideoController } from './useVideo';
import VideoDiagnostics from './VideoDiagnostics';
import RecentVideos from './RecentVideos';

const phaseLabels: Record<string, string> = {
  prepared: '已准备',
  switching: '正在切换播放',
  starting: '正在发送影片',
  awaiting_evidence: '等待电视播放反馈',
  playing: '电视上报正在播放',
  paused: '已暂停',
  remote_state_unknown: '电视播放状态待确认',
  ended: '播放已结束',
  stopped: '已停止',
  failed: '播放未完成',
  detached: '电视已切换到其他内容',
};

export function sessionPhase(session: VideoSession) {
  return phaseLabels[session.phase] || '电视播放状态待确认';
}

function videoSize(value: string) {
  try {
    const bytes = BigInt(value);
    const unit = 1024n * 1024n * 1024n;
    if (bytes >= unit) return Number((bytes * 10n) / unit) / 10 + ' GiB';
    return Number((bytes * 10n) / (1024n * 1024n)) / 10 + ' MiB';
  } catch {
    return '大小未知';
  }
}

function FileSummary({ entry }: { entry: VideoInfo }) {
  return (
    <div className="video-file-summary">
      <strong>{entry.title}</strong>
      <dl className="video-facts">
        <div>
          <dt>文件</dt>
          <dd>
            {entry.format.toUpperCase()} · {videoSize(entry.size)}
          </dd>
        </div>
        <div>
          <dt>画面</dt>
          <dd>
            {entry.width && entry.height ? entry.width + ' × ' + entry.height : '分辨率未知'}
            {' · '}
            {entry.video_codec || '编码未知'}
          </dd>
        </div>
        <div>
          <dt>时长</dt>
          <dd>{entry.duration_ms === null ? '未知' : formatTime(entry.duration_ms / 1000)}</dd>
        </div>
        <div>
          <dt>色彩</dt>
          <dd>{entry.color_transfer || '未识别'} · 电视 HDR 输出待确认</dd>
        </div>
      </dl>
      <details className="video-track-details">
        <summary>音轨与字幕信息</summary>
        <p className="field-help">
          可用电视遥控器切换音轨。需要指定字幕时，在「投送字幕」里选择轨道。
        </p>
        <h4>音轨</h4>
        {entry.audio_tracks.length ? (
          <ul>
            {entry.audio_tracks.map((track) => (
              <li key={track.index}>
                {[
                  track.language || '语言未知',
                  track.codec || '编码未知',
                  track.title,
                  track.channels ? track.channels + ' 声道' : '',
                ]
                  .filter(Boolean)
                  .join(' · ')}
              </li>
            ))}
          </ul>
        ) : (
          <p>未识别音轨，实际声音需要在电视上确认。</p>
        )}
        <h4>字幕</h4>
        {entry.subtitle_tracks.length ? (
          <ul>
            {entry.subtitle_tracks.map((track) => (
              <li key={track.index}>
                {[track.language || '语言未知', track.codec || '格式未知', track.title]
                  .filter(Boolean)
                  .join(' · ')}
              </li>
            ))}
          </ul>
        ) : (
          <p>未识别内封字幕。</p>
        )}
      </details>
    </div>
  );
}

export default function VideoPanel({
  invoke,
  visible,
  video,
  devices,
  busy,
  connectionError,
  reconnect,
  discovering,
  discover,
  addDevice,
}: {
  invoke: Invoke;
  visible: boolean;
  video: VideoController;
  devices: VideoDevice[];
  busy: boolean;
  connectionError: string;
  reconnect: () => void;
  discovering: boolean;
  discover: () => void;
  addDevice: () => void;
}) {
  const { state, error, pending, run } = video;
  const [candidate, setCandidate] = useState('');
  const [confirm, setConfirm] = useState(false);
  const candidateRef = useRef<HTMLButtonElement | null>(null);
  const sourceHeading = useRef<HTMLHeadingElement>(null);
  const unavailable = busy || pending || !state.enabled;
  const probing = state.probe?.status === 'running' || state.probe?.status === 'probing';
  const entry = state.entry;
  const [selection, setSelection] = useState<{
    id: string;
    choice: SubtitleSelection | null;
  } | null>(null);
  const choice = selection && selection.id === entry?.id ? selection.choice : null;
  const processing = state.probe?.status === 'subtitles';
  const sameSubtitle =
    (state.plan?.subtitle?.index ?? null) === (choice?.index ?? null) &&
    state.plan?.subtitle?.mode === choice?.mode;
  const targetExists = devices.some((device) => device.id === candidate);
  const plan =
    state.plan?.media_id === entry?.id && state.plan?.device_id === candidate && sameSubtitle
      ? state.plan
      : null;
  const active =
    state.session && !['stopped', 'ended', 'failed', 'detached'].includes(state.session.phase);
  const replacing = state.music_active || Boolean(active) || state.owner_kind === 'unknown';
  const errors = [
    ...new Set(
      [error, state.error, state.probe?.error, state.session?.error]
        .filter(Boolean)
        .map(videoMessage),
    ),
  ];
  const start = async (replaceCurrent: boolean) => {
    if (!plan || unavailable) return;
    const result = await run('start_video_play', {
      planId: plan.id,
      expectedOwnerRevision: state.owner_revision,
      replaceCurrent,
    });
    if (result) setConfirm(false);
  };
  return (
    <main className="video-workspace" aria-labelledby="video-title">
      <div className="library-heading">
        <div>
          <span className="eyebrow">本地影片</span>
          <h1 id="video-title" tabIndex={-1}>
            影视投送
          </h1>
          <p>选择电脑上的影片，在电视上播放。选片和检查信息不会停止音乐。</p>
        </div>
      </div>
      {connectionError && (
        <div className="notice warning video-notice" role="status">
          <Icon name="info" />
          <p>{connectionError}</p>
          <button type="button" className="text-button" onClick={reconnect}>
            重新检查客户端
          </button>
        </div>
      )}
      {errors.length > 0 && (
        <div className="notice warning video-notice" role="alert">
          <Icon name="info" />
          <div>
            <p>{errors.join(' ')}</p>
            <button
              type="button"
              className="text-button"
              disabled={busy || pending}
              onClick={() => void video.refresh()}
            >
              重新读取状态
            </button>
          </div>
        </div>
      )}
      <div className="video-columns">
        <section className="video-source" aria-labelledby="video-source-title">
          <div className="video-section-heading">
            <h2 id="video-source-title" ref={sourceHeading} tabIndex={-1}>
              影片文件
            </h2>
            <button
              type="button"
              className="button outline"
              disabled={unavailable || probing}
              aria-busy={pending}
              onClick={() => void run('pick_video_file')}
            >
              <Icon name="folder" />
              <span>{entry ? '选择其他影片' : '选择影片'}</span>
            </button>
          </div>
          {probing && (
            <div className="video-probe" role="status">
              <p>正在读取影片信息，不会复制或转换原文件。</p>
              <button
                type="button"
                className="text-button"
                disabled={unavailable}
                onClick={() => void run('cancel_video_probe', { taskId: state.probe?.task_id })}
              >
                取消读取
              </button>
            </div>
          )}
          {entry ? (
            <FileSummary entry={entry} />
          ) : (
            <div className="video-empty">
              <Icon name="folder" />
              <h3>从一个影片文件开始</h3>
              <p>支持选择 MKV、MP4、M2TS 单文件。实际播放能力需要在电视上验证。</p>
            </div>
          )}
        </section>
        <section className="video-target" aria-labelledby="video-target-title">
          <h2 id="video-target-title">播放到电视</h2>
          <label className="field-label" htmlFor="video-device">
            候选电视
          </label>
          <Select
            id="video-device"
            aria-label="候选电视"
            value={candidate}
            disabled={unavailable}
            onChange={(event: { target: { value: string } }) => {
              setCandidate(event.target.value);
              setConfirm(false);
            }}
          >
            <option value="">请选择电视</option>
            {devices.map((device) => (
              <option key={device.id} value={device.id}>
                {device.name}
              </option>
            ))}
          </Select>
          <p className="field-help" id="video-device-help">
            这里只选择候选设备，点击播放后才会切换当前内容。请确认选中的是电视。
          </p>
          <div className="video-device-actions">
            <button
              type="button"
              className="text-button"
              disabled={unavailable || discovering}
              onClick={discover}
            >
              {discovering ? '正在搜索设备…' : '重新搜索设备'}
            </button>
            <button
              type="button"
              className="text-button"
              disabled={unavailable}
              onClick={addDevice}
            >
              通过地址添加
            </button>
          </div>
          {!devices.length && (
            <p className="field-help">
              暂无设备。请开启电视接收服务，并确认电脑与电视在同一局域网。
            </p>
          )}
          {candidate && !targetExists && (
            <p className="field-error">候选设备已离线，请重新搜索或选择。</p>
          )}
          {entry && (
            <SubtitleOptions
              entry={entry}
              choice={choice}
              disabled={unavailable || processing}
              onChange={(next) => {
                setSelection({ id: entry.id, choice: next });
                setConfirm(false);
              }}
            />
          )}
          {processing && (
            <div className="video-probe" role="status">
              <p>
                正在准备字幕版本
                {state.probe?.percent != null ? ' · ' + state.probe.percent + '%' : ''}
              </p>
              <button
                type="button"
                className="text-button"
                disabled={unavailable}
                onClick={() => void run('cancel_video_probe', { taskId: state.probe?.task_id })}
              >
                取消字幕处理
              </button>
            </div>
          )}
          <button
            ref={candidateRef}
            type="button"
            className="button outline video-prepare"
            disabled={unavailable || probing || processing || !entry || !targetExists}
            onClick={() =>
              void run('prepare_video_play', {
                mediaId: entry?.id,
                deviceId: candidate,
                ...(choice ? { subtitle: choice } : {}),
              })
            }
          >
            检查并准备
          </button>
          <p className="field-help">
            {choice ? '准备完成后即可播放。' : '仅检查文件与设备，不会发送播放指令。'}
          </p>
          {plan && (
            <div className="video-plan" role="status">
              <h3>
                {plan.mode === 'blocked'
                  ? '暂时无法播放'
                  : plan.subtitle
                    ? '字幕版本已准备'
                    : '原文件直传'}
              </h3>
              <p>{plan.reason}</p>
              {plan.mode !== 'blocked' && (
                <>
                  <p className="field-help">
                    {plan.subtitle
                      ? '已选择字幕轨道 ' + plan.subtitle.index + '，可在电视上确认效果。'
                      : '视频和音频保持原文件内容，电视的实际画面、声音和字幕仍需确认。'}
                  </p>
                  <button
                    type="button"
                    className="button primary"
                    disabled={unavailable}
                    onClick={() => (replacing ? setConfirm(true) : void start(false))}
                  >
                    {replacing
                      ? '切换并播放影片'
                      : plan.subtitle
                        ? '播放字幕版本'
                        : plan.mode === 'trial'
                          ? '试播原文件'
                          : '播放影片'}
                  </button>
                </>
              )}
            </div>
          )}
        </section>
      </div>
      {state.session && (
        <section className="video-evidence" aria-labelledby="video-evidence-title">
          <h2 id="video-evidence-title">播放反馈</h2>
          <p>
            {state.session.title} · {sessionPhase(state.session)}
          </p>
          <ul>
            <li>播放指令：{state.session.command_accepted ? '电视已接受' : '尚未确认'}</li>
            <li>
              {state.session.subtitle ? '字幕版本' : '原文件'}：
              {state.session.http_requested ? '电视已请求读取' : '尚未收到读取请求'}
            </li>
            {state.session.subtitle && (
              <li>
                字幕轨道 {state.session.subtitle.index} ·{' '}
                {state.session.subtitle.mode === 'remux'
                  ? '默认、强制字幕'
                  : state.session.subtitle.mode === 'burn_hdr10'
                    ? '已合入原分辨率 HDR10 画面'
                    : '已合入 1080p 画面'}
              </li>
            )}
            <li>画面和声音：请在电视上确认，读取数据不等于已经正常播放。</li>
          </ul>
          <VideoDiagnostics
            key={state.session.id + ':' + state.session.generation}
            video={video}
            session={state.session}
            unavailable={unavailable}
          />
        </section>
      )}
      <MagnetDownload
        invoke={invoke}
        visible={visible}
        ready={state.enabled && !connectionError}
        canOpen={!unavailable && !probing && !processing}
        openFile={async (id, index) => {
          if (await run('video_magnet_open', { id, index })) sourceHeading.current?.focus();
        }}
      />
      <RecentVideos
        video={video}
        unavailable={unavailable}
        probing={probing}
        onSelected={() => sourceHeading.current?.focus()}
      />
      <p className="local-note">
        <span className="status-dot" />
        投送期间请保持电脑和应用运行。
      </p>
      <Dialog
        id="video-switch-dialog"
        title="切换到影片"
        eyebrow="切换当前播放"
        description="video-switch-description"
        open={confirm}
        busy={unavailable}
        onClose={() => setConfirm(false)}
      >
        <div className="dialog-body">
          <p id="video-switch-description">
            将停止当前
            {state.owner_kind === 'unknown' ? '内容' : state.music_active ? '音乐' : '影片'}
            ，并在候选电视播放「{entry?.title}」。 如果切换失败，应用不会自动重播原内容。
          </p>
          <p className="field-error" role="alert">
            {error ||
              (state.error || state.session?.error
                ? videoMessage(state.error || state.session?.error)
                : '')}
          </p>
          <div className="dialog-actions">
            <button
              type="button"
              className="button outline"
              data-autofocus
              disabled={unavailable}
              onClick={() => setConfirm(false)}
            >
              取消
            </button>
            <button
              type="button"
              className="button primary"
              disabled={unavailable}
              aria-busy={pending}
              onClick={() => void start(true)}
            >
              {pending ? '正在切换…' : '停止当前内容并播放'}
            </button>
          </div>
        </div>
      </Dialog>
    </main>
  );
}

export function VideoPlayer({
  video,
  busy,
  onOpen,
}: {
  video: VideoController;
  busy: boolean;
  onOpen: () => void;
}) {
  const session = video.state.session;
  useEffect(() => {
    if (session) document.title = session.title + ' | 影视投送';
  }, [session?.title]);
  if (!session) return null;
  const unavailable = busy || video.pending;
  const paused = session.phase === 'paused';
  const playing = session.phase === 'playing';
  const ended = ['ended', 'stopped', 'failed', 'detached'].includes(session.phase);
  const control = (action: string, positionMs?: number) =>
    video.run('control_video', {
      sessionId: session.id,
      expectedGeneration: session.generation,
      action,
      ...(positionMs === undefined ? {} : { positionMs }),
    });
  return (
    <footer className="player video-player" aria-label="影视播放控制">
      <div className="now-playing">
        <div className="album-icon">
          <Icon name="play" />
        </div>
        <div className="now-copy">
          <strong>{session.title}</strong>
          <span>{sessionPhase(session)}</span>
        </div>
      </div>
      <div className="transport">
        <div className="transport-buttons">
          <button
            type="button"
            className="button outline"
            disabled={unavailable || !session.can_pause || (!playing && !paused)}
            onClick={() => void control(paused ? 'resume' : 'pause')}
          >
            {paused ? '继续影片' : '暂停影片'}
          </button>
          <button
            type="button"
            className="button outline"
            disabled={unavailable || ended}
            onClick={() => void control('stop')}
          >
            停止影片
          </button>
        </div>
        <div className="timeline">
          <Range
            id="video-seek"
            value={Math.min(session.position_ms || 0, session.duration_ms || 0) / 1000}
            max={(session.duration_ms || 0) / 1000}
            disabled={
              unavailable || !session.can_seek || !session.duration_ms || (!playing && !paused)
            }
            onCommit={(seconds: number) => control('seek', Math.round(seconds * 1000))}
            time
          />
        </div>
      </div>
      <div className="video-player-details">
        <span>{session.can_seek ? '拖动按设备能力执行' : '此设备尚未确认支持拖动'}</span>
        <button type="button" className="text-button" onClick={onOpen}>
          查看影片与状态
        </button>
      </div>
    </footer>
  );
}
