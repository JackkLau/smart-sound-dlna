import { useState } from 'react';
import { formatTime } from '../utils.js';
import type { VideoSession } from './types';
import type { VideoController } from './useVideo';

const actions: Record<string, string> = {
  SetAVTransportURI: '发送影片地址',
  Play: '播放',
  Pause: '暂停',
  Stop: '停止',
  Seek: '跳转',
  pause: '暂停',
  resume: '继续',
  stop: '停止',
  seek: '跳转',
};
const outcomes = { pending: '等待确认', accepted: '设备已接受', failed: '未确认完成' };
const sources = { command: '本次操作结果', poll: '电视状态查询', unknown: '尚未取得状态' };

function byteCount(value: string) {
  try {
    return BigInt(value).toLocaleString('zh-CN') + ' 字节';
  } catch {
    return '未知';
  }
}

export default function VideoDiagnostics({
  video,
  session,
  unavailable,
}: {
  video: VideoController;
  session: VideoSession;
  unavailable: boolean;
}) {
  const [saved, setSaved] = useState(false);
  const diagnostics = session.diagnostics;
  const canRecheck =
    session.generation === video.state.owner_revision &&
    ['video', 'unknown'].includes(video.state.owner_kind || '') &&
    !['stopped', 'ended', 'failed', 'detached'].includes(session.phase);
  const save = async () => {
    setSaved(false);
    if ((await video.exportDiagnostics(session)) === true) setSaved(true);
  };
  return (
    <>
      <div className="video-device-actions">
        <button
          type="button"
          className="button outline"
          disabled={unavailable || !canRecheck}
          aria-busy={video.pending}
          onClick={() =>
            void video.run('recheck_video_session', {
              sessionId: session.id,
              expectedGeneration: session.generation,
            })
          }
        >
          重新检查连接
        </button>
        <button
          type="button"
          className="button outline"
          disabled={unavailable}
          aria-busy={video.pending}
          onClick={() => void save()}
        >
          导出诊断
        </button>
      </div>
      <p className="field-help">
        检查连接不会重新播放。网络或电视地址变化后，先重新搜索设备，再检查连接；需要再次播放时重新准备影片。
      </p>
      <p className="field-help" role="status">
        {saved ? '诊断已保存。' : ' '}
      </p>
      <details className="video-track-details">
        <summary>连接与传输诊断</summary>
        <p className="field-help">
          数据量是本应用向 HTTP 响应提供的字节，重复读取会累计，不代表电视已收完或已播放。
          导出只保存本次会话的脱敏诊断，不包含文件路径、媒体链接或账号凭据。
        </p>
        {diagnostics ? (
          <dl className="video-facts">
            <div>
              <dt>已提供数据</dt>
              <dd>{byteCount(diagnostics.transfer.bytes_streamed)}</dd>
            </div>
            <div>
              <dt>读取请求</dt>
              <dd>
                GET {diagnostics.transfer.get_requests} · HEAD {diagnostics.transfer.head_requests}{' '}
                · Range {diagnostics.transfer.range_requests}
              </dd>
            </div>
            <div>
              <dt>最近读取</dt>
              <dd>
                {diagnostics.transfer.last_request
                  ? `${diagnostics.transfer.last_request.method} · HTTP ${diagnostics.transfer.last_request.status} · ${
                      diagnostics.transfer.last_request.range_start !== null &&
                      diagnostics.transfer.last_request.range_end !== null
                        ? diagnostics.transfer.last_request.range_start +
                          '–' +
                          diagnostics.transfer.last_request.range_end +
                          ' 字节'
                        : '无字节范围'
                    }`
                  : '尚无读取请求'}
              </dd>
            </div>
            <div>
              <dt>文件服务</dt>
              <dd>
                {diagnostics.transfer.revoked ? '已关闭本次授权' : '本次授权有效'} · 当前{' '}
                {diagnostics.transfer.active_streams} 路 · 峰值{' '}
                {diagnostics.transfer.peak_active_streams} 路 · 拒绝{' '}
                {diagnostics.transfer.rejected_requests} 次
              </dd>
            </div>
            <div>
              <dt>最近指令</dt>
              <dd>
                {diagnostics.last_command
                  ? `${actions[diagnostics.last_command.action] || diagnostics.last_command.action} · ${outcomes[diagnostics.last_command.outcome]} · ${diagnostics.last_command.elapsed_ms} 毫秒`
                  : '尚无指令记录'}
              </dd>
            </div>
            <div>
              <dt>状态来源</dt>
              <dd>
                {sources[diagnostics.state_source]} · {session.transport_state || '未知'}
              </dd>
            </div>
            <div>
              <dt>连接观测</dt>
              <dd>
                {diagnostics.last_poll_age_ms === null
                  ? '尚无状态查询记录'
                  : `最近查询 ${Math.floor(diagnostics.last_poll_age_ms / 1000)} 秒前`}{' '}
                · 连续失败 {diagnostics.consecutive_poll_failures} 次
              </dd>
            </div>
            <div>
              <dt>本次会话</dt>
              <dd>{formatTime(diagnostics.elapsed_ms / 1000)}</dd>
            </div>
          </dl>
        ) : (
          <p className="field-help">此会话尚无详细诊断，请重新读取状态。</p>
        )}
      </details>
    </>
  );
}
