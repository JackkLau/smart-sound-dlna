export type VideoTrackInfo = {
  index: number;
  codec: string | null;
  language: string | null;
  title: string | null;
  channels: number | null;
};

export type VideoInfo = {
  id: string;
  title: string;
  format: string;
  size: string;
  duration_ms: number | null;
  video_codec: string | null;
  width: number | null;
  height: number | null;
  pixel_format: string | null;
  color_transfer: string | null;
  can_burn_hdr10: boolean;
  audio_tracks: VideoTrackInfo[];
  subtitle_tracks: VideoTrackInfo[];
};

export type SubtitleSelection = { index: number; mode: 'remux' | 'burn' | 'burn_hdr10' };

export type VideoPlan = {
  subtitle?: SubtitleSelection | null;
  id: string;
  media_id: string;
  device_id: string;
  mode: string;
  reason: string;
  can_pause: boolean;
  can_seek: boolean;
};

export type VideoDiagnostics = {
  schema_version: number;
  elapsed_ms: number;
  last_command: {
    action: string;
    outcome: 'pending' | 'accepted' | 'failed';
    elapsed_ms: number;
  } | null;
  last_poll_age_ms: number | null;
  consecutive_poll_failures: number;
  state_source: 'command' | 'poll' | 'unknown';
  transfer: {
    get_requests: number;
    head_requests: number;
    range_requests: number;
    rejected_requests: number;
    active_streams: number;
    peak_active_streams: number;
    bytes_streamed: string;
    last_request: {
      method: string;
      status: number;
      range_start: string | null;
      range_end: string | null;
    } | null;
    revoked: boolean;
  };
};

export type VideoSession = {
  subtitle?: SubtitleSelection | null;
  id: string;
  generation: number;
  media_id: string;
  device_id: string;
  title: string;
  phase: string;
  position_ms: number | null;
  duration_ms: number | null;
  can_pause: boolean;
  can_seek: boolean;
  command_accepted: boolean;
  http_requested: boolean;
  transport_state: string;
  diagnostics?: VideoDiagnostics;
  error: string | null;
};

export type RecentVideo = {
  id: string;
  title: string;
  format: string;
  size: string;
  duration_ms: number | null;
  video_codec: string | null;
  last_selected_ms: number;
  progress: { position_ms: number; duration_ms: number; observed_at_ms: number } | null;
};

export type VideoSnapshot = {
  recent_videos?: RecentVideo[];
  catalog_warning?: string | null;
  enabled: boolean;
  owner_kind?: string;
  entry: VideoInfo | null;
  probe: { task_id: string; status: string; error: string | null; percent?: number | null } | null;
  plan: VideoPlan | null;
  session: VideoSession | null;
  owner_revision: number;
  music_active: boolean;
  error: string | null;
};

export type VideoDevice = { id: string; name: string; address?: string; model?: string };
export type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
export type Mutate = (
  command: string,
  args?: Record<string, unknown>,
  successMessage?: string,
  options?: { scopedError: boolean },
) => Promise<unknown>;
