export type RealtimeChannel = "microphone" | "system";

export type RealtimeMode = "transcribe" | "translate";

export type RealtimeStatus =
  | "starting"
  | "listening"
  | "stopping"
  | "idle"
  | "error";

export type RealtimeEventKind =
  | "status"
  | "input_transcript_delta"
  | "input_transcript_completed"
  | "output_transcript_delta"
  | "output_transcript_completed"
  | "level"
  | "error";

export interface AudioDevice {
  id: string;
  name: string;
  isDefault: boolean;
  channels: number;
  sampleRate: number;
  monitoredOutputId?: string | null;
  excludesCurrentProcessAudio?: boolean;
}

export interface AudioDevices {
  inputs: AudioDevice[];
  outputs: AudioDevice[];
  systemSources: AudioDevice[];
}

export interface OpenAiKeyStatus {
  configured: boolean;
}

export interface RealtimeEvent {
  channel: RealtimeChannel;
  kind: RealtimeEventKind;
  segmentId?: string;
  delta?: string;
  text?: string;
  status?: RealtimeStatus;
  level?: number;
  message?: string;
}

export interface ChannelSettings {
  enabled: boolean;
  mode: RealtimeMode;
  playbackEnabled: boolean;
  inputDeviceId: string | null;
  outputDeviceId: string | null;
  targetLanguage: string;
}

export interface WorkspaceSettings {
  microphone: ChannelSettings;
  system: ChannelSettings;
}

export interface RealtimeStartRequest {
  channel: RealtimeChannel;
  mode: RealtimeMode;
  playbackEnabled: boolean;
  inputDeviceId?: string | null;
  outputDeviceId?: string | null;
  targetLanguage?: string | null;
}
