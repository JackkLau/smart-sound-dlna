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
  audio_tracks: VideoTrackInfo[];
  subtitle_tracks: VideoTrackInfo[];
};

export type VideoPlan = {
  id: string;
  media_id: string;
  device_id: string;
  mode: string;
  reason: string;
  can_pause: boolean;
  can_seek: boolean;
};

export type VideoSession = {
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
  error: string | null;
};

export type VideoSnapshot = {
  enabled: boolean;
  owner_kind?: string;
  entry: VideoInfo | null;
  probe: { task_id: string; status: string; error: string | null } | null;
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
