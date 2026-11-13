import { useEffect, useRef, useState } from 'react';
import { Dialog } from '../components.jsx';
import { formatTime } from '../utils.js';
import type { RecentVideo } from './types';
import type { VideoController } from './useVideo';

export default function RecentVideos({
  video,
  unavailable,
  probing,
  onSelected,
}: {
  video: VideoController;
  unavailable: boolean;
  probing: boolean;
  onSelected: () => void;
}) {
  const [limit, setLimit] = useState(5);
  const [removing, setRemoving] = useState<RecentVideo | null>(null);
  const [feedback, setFeedback] = useState('');
  const heading = useRef<HTMLHeadingElement>(null);
  const entries = video.state.recent_videos || [];
  useEffect(() => {
    if (feedback) heading.current?.focus();
  }, [feedback]);
  const forget = async () => {
    if (!removing || unavailable) return;
    setFeedback('');
    if (await video.run('forget_recent_video', { mediaId: removing.id })) {
      setRemoving(null);
      setFeedback('影片记录已移除。');
    }
  };
  const select = async (entry: RecentVideo) => {
    setFeedback('');
    if (await video.run('select_recent_video', { mediaId: entry.id })) onSelected();
  };
  return (
    <section className="video-evidence video-recent" aria-labelledby="video-recent-title">
      <h2 ref={heading} id="video-recent-title" tabIndex={-1}>
        最近影片
      </h2>
      <p className="field-help">
        保留最近 100 部影片。重新选择会核对文件并分析，不会自动播放或续播。
      </p>
      <p className="field-help">
        位置来自电视播放反馈，可能比当前进度稍早；正常保存时，异常退出可能丢失最近 30 秒的记录。
      </p>
      <p className="field-help" role="status">
        {feedback}
      </p>
      {video.state.catalog_warning && (
        <p className="field-error" role="status">
          {video.state.catalog_warning}
        </p>
      )}
      {entries.length ? (
        <>
          <ul className="video-recent-list">
            {entries.slice(0, limit).map((entry) => (
              <li key={entry.id}>
                <div className="video-recent-info">
                  <strong>{entry.title}</strong>
                  <p className="field-help">
                    {entry.format.toUpperCase()} · {entry.video_codec || '编码未知'}
                  </p>
                  <p className="field-help">
                    {entry.progress
                      ? `上次确认位置 ${formatTime(entry.progress.position_ms / 1000)} / ${formatTime(entry.progress.duration_ms / 1000)}`
                      : '尚无确认的播放位置'}
                  </p>
                </div>
                <div className="video-recent-actions">
                  <button
                    type="button"
                    className="button outline"
                    aria-label={'重新选择 ' + entry.title}
                    disabled={unavailable || probing}
                    onClick={() => void select(entry)}
                  >
                    重新选择
                  </button>
                  <button
                    type="button"
                    className="text-button"
                    aria-label={'移除记录 ' + entry.title}
                    disabled={unavailable || probing}
                    onClick={() => {
                      setFeedback('');
                      setRemoving(entry);
                    }}
                  >
                    移除记录
                  </button>
                </div>
              </li>
            ))}
          </ul>
          <div className="video-recent-footer">
            <span className="field-help">
              已显示 {Math.min(limit, entries.length)} / {entries.length} 部
            </span>
            {limit < entries.length && (
              <button
                type="button"
                className="text-button"
                onClick={() => setLimit((value) => value + 5)}
              >
                显示更多影片
              </button>
            )}
          </div>
        </>
      ) : (
        <p className="field-help">暂无影片记录。选择影片并完成分析后会保存在这里。</p>
      )}
      <Dialog
        id="video-forget-dialog"
        title="移除影片记录"
        eyebrow="最近影片"
        description="video-forget-description"
        open={Boolean(removing)}
        busy={unavailable}
        onClose={() => setRemoving(null)}
      >
        <div className="dialog-body">
          <p id="video-forget-description">
            将移除「{removing?.title}」的历史与播放位置，不会删除原文件或停止当前播放。
          </p>
          <p className="field-error" role="alert">
            {video.error}
          </p>
          <div className="dialog-actions">
            <button
              type="button"
              className="button outline"
              data-autofocus
              disabled={unavailable}
              onClick={() => setRemoving(null)}
            >
              取消
            </button>
            <button
              type="button"
              className="button danger"
              disabled={unavailable}
              aria-busy={video.pending}
              onClick={() => void forget()}
            >
              移除记录
            </button>
          </div>
        </div>
      </Dialog>
    </section>
  );
}
