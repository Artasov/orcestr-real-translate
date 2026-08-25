import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AudioDevices,
  OpenAiKeyStatus,
  RealtimeEvent,
  RealtimeStartRequest,
} from "./types";

export const nativeRealtime = {
  keyStatus: () => invoke<OpenAiKeyStatus>("openai_key_status"),
  saveKey: (apiKey: string) =>
    invoke<OpenAiKeyStatus>("openai_key_save", { apiKey }),
  deleteKey: () => invoke<OpenAiKeyStatus>("openai_key_delete"),
  listDevices: () => invoke<AudioDevices>("audio_list_devices"),
  start: (request: RealtimeStartRequest) =>
    invoke<void>("realtime_start", { request }),
  setPlaybackEnabled: (
    channel: RealtimeStartRequest["channel"],
    enabled: boolean,
  ) => invoke<void>("realtime_set_playback_enabled", { channel, enabled }),
  setPlaybackVolume: (
    channel: RealtimeStartRequest["channel"],
    volumeDb: number,
  ) => invoke<void>("realtime_set_playback_volume", { channel, volumeDb }),
  stop: (channel: RealtimeStartRequest["channel"]) =>
    invoke<void>("realtime_stop", { channel }),
  stopAll: () => invoke<void>("realtime_stop_all"),
  onEvent: (handler: (event: RealtimeEvent) => void): Promise<UnlistenFn> =>
    listen<RealtimeEvent>("realtime:event", ({ payload }) => handler(payload)),
};
