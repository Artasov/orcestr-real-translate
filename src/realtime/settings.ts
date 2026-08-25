import type {
  AudioDevice,
  AudioDevices,
  ChannelSettings,
  RealtimeChannel,
  WorkspaceSettings,
} from "./types";

export const WORKSPACE_SETTINGS_KEY =
  "orcestr-real-translate:workspace-settings:v3";
export const WINDOWS_PROCESS_LOOPBACK_ID = "windows-process-loopback";
export const PLAYBACK_VOLUME_MIN_DB = -24;
export const PLAYBACK_VOLUME_MAX_DB = 12;
export const PLAYBACK_VOLUME_DEFAULT_DB = 0;

export const DEFAULT_WORKSPACE_SETTINGS: WorkspaceSettings = {
  microphone: {
    enabled: true,
    mode: "translate",
    playbackEnabled: true,
    playbackVolumeDb: PLAYBACK_VOLUME_DEFAULT_DB,
    inputDeviceId: null,
    outputDeviceId: null,
    targetLanguage: "en",
  },
  system: {
    enabled: false,
    mode: "translate",
    playbackEnabled: true,
    playbackVolumeDb: PLAYBACK_VOLUME_DEFAULT_DB,
    inputDeviceId: null,
    outputDeviceId: null,
    targetLanguage: "ru",
  },
};

interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export interface SettingsValidation {
  valid: boolean;
  code: SettingsValidationCode | null;
  channel: RealtimeChannel | null;
  feedbackRisk: boolean;
  feedbackConflict: FeedbackConflict | null;
}

export type SettingsValidationCode =
  | "api_key_required"
  | "channel_required"
  | "microphone_unavailable"
  | "system_unavailable"
  | "source_unavailable"
  | "output_unavailable"
  | "selected_output_unavailable"
  | "language_required"
  | "feedback";

export type ApiKeyValidationCode = "empty" | "whitespace" | "too_long";

export interface FeedbackConflict {
  outputChannel: RealtimeChannel;
  source: AudioDevice;
  output: AudioDevice;
}

export function loadWorkspaceSettings(
  storage: Pick<StorageLike, "getItem"> | null = browserStorage(),
): WorkspaceSettings {
  if (!storage) return cloneDefaults();
  try {
    const raw = storage.getItem(WORKSPACE_SETTINGS_KEY);
    if (!raw) return cloneDefaults();
    return normalizeWorkspaceSettings(JSON.parse(raw) as unknown);
  } catch {
    return cloneDefaults();
  }
}

export function saveWorkspaceSettings(
  settings: WorkspaceSettings,
  storage: Pick<StorageLike, "setItem"> | null = browserStorage(),
): boolean {
  if (!storage) return false;
  try {
    storage.setItem(
      WORKSPACE_SETTINGS_KEY,
      JSON.stringify(normalizeWorkspaceSettings(settings)),
    );
    return true;
  } catch {
    return false;
  }
}

export function normalizeWorkspaceSettings(value: unknown): WorkspaceSettings {
  if (!isRecord(value)) return cloneDefaults();
  return {
    microphone: normalizeChannel(
      value.microphone,
      DEFAULT_WORKSPACE_SETTINGS.microphone,
    ),
    system: normalizeChannel(value.system, DEFAULT_WORKSPACE_SETTINGS.system),
  };
}

export function validateWorkspaceStart(
  settings: WorkspaceSettings,
  devices: AudioDevices,
  keyConfigured: boolean,
): SettingsValidation {
  const enabled = (["microphone", "system"] as const).filter(
    (channel) => settings[channel].enabled,
  );
  if (!keyConfigured) {
    return invalid("api_key_required");
  }
  if (enabled.length === 0) {
    return invalid("channel_required");
  }

  for (const channel of enabled) {
    const channelSettings = settings[channel];
    const sources =
      channel === "microphone" ? devices.inputs : devices.systemSources;
    if (sources.length === 0) {
      return invalid(
        channel === "microphone"
          ? "microphone_unavailable"
          : "system_unavailable",
        channel,
      );
    }
    if (
      channelSettings.inputDeviceId &&
      !sources.some((device) => device.id === channelSettings.inputDeviceId)
    ) {
      return invalid("source_unavailable", channel);
    }
    if (
      channelSettings.mode === "translate" &&
      channelSettings.playbackEnabled &&
      devices.outputs.length === 0
    ) {
      return invalid("output_unavailable", channel);
    }
    if (
      channelSettings.mode === "translate" &&
      channelSettings.playbackEnabled &&
      channelSettings.outputDeviceId &&
      !devices.outputs.some(
        (device) => device.id === channelSettings.outputDeviceId,
      )
    ) {
      return invalid("selected_output_unavailable", channel);
    }
    if (
      channelSettings.mode === "translate" &&
      !channelSettings.targetLanguage.trim()
    ) {
      return invalid("language_required", channel);
    }
  }

  const feedbackConflict = systemFeedbackConflict(settings, devices);
  if (feedbackConflict) {
    return {
      valid: false,
      code: "feedback",
      channel: "system",
      feedbackRisk: true,
      feedbackConflict,
    };
  }

  return {
    valid: true,
    code: null,
    channel: null,
    feedbackRisk: false,
    feedbackConflict: null,
  };
}

export function systemFeedbackRisk(
  settings: WorkspaceSettings,
  devices: AudioDevices,
): boolean {
  return systemFeedbackConflict(settings, devices) !== null;
}

export function systemFeedbackConflict(
  settings: WorkspaceSettings,
  devices: AudioDevices,
): FeedbackConflict | null {
  if (!settings.system.enabled) return null;
  const source = selectedOrDefault(
    devices.systemSources,
    settings.system.inputDeviceId,
  );
  if (!source) return null;
  if (
    source.excludesCurrentProcessAudio ||
    source.id === WINDOWS_PROCESS_LOOPBACK_ID
  ) {
    return null;
  }

  for (const outputChannel of ["system", "microphone"] as const) {
    const channel = settings[outputChannel];
    if (
      !channel.enabled ||
      channel.mode !== "translate" ||
      !channel.playbackEnabled
    ) {
      continue;
    }
    const output = selectedOrDefault(devices.outputs, channel.outputDeviceId);
    const capturedOutputId = source.monitoredOutputId ?? source.id;
    if (output?.id === capturedOutputId) {
      return { outputChannel, source, output };
    }
  }
  return null;
}

export function resolveDeviceId(
  devices: readonly AudioDevice[],
  selectedId: string | null,
): string | null {
  return selectedOrDefault(devices, selectedId)?.id ?? null;
}

export function validateApiKeyDraft(
  value: string,
): ApiKeyValidationCode | null {
  const key = value.trim();
  if (!key) return "empty";
  if (/\s/u.test(key)) return "whitespace";
  if (key.length > 8 * 1_024) return "too_long";
  return null;
}

function normalizeChannel(
  value: unknown,
  fallback: ChannelSettings,
): ChannelSettings {
  if (!isRecord(value)) return { ...fallback };
  return {
    enabled:
      typeof value.enabled === "boolean" ? value.enabled : fallback.enabled,
    mode:
      value.mode === "transcribe" || value.mode === "translate"
        ? value.mode
        : fallback.mode,
    playbackEnabled:
      typeof value.playbackEnabled === "boolean"
        ? value.playbackEnabled
        : fallback.playbackEnabled,
    playbackVolumeDb: normalizePlaybackVolume(value.playbackVolumeDb),
    inputDeviceId: nullableId(value.inputDeviceId),
    outputDeviceId: nullableId(value.outputDeviceId),
    targetLanguage: languageCode(value.targetLanguage, fallback.targetLanguage),
  };
}

export function normalizePlaybackVolume(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return PLAYBACK_VOLUME_DEFAULT_DB;
  }
  return Math.round(
    Math.min(PLAYBACK_VOLUME_MAX_DB, Math.max(PLAYBACK_VOLUME_MIN_DB, value)),
  );
}

function nullableId(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 && value.length <= 2_048
    ? value
    : null;
}

function languageCode(value: unknown, fallback: string): string {
  return typeof value === "string" && /^[a-z]{2,3}(?:-[A-Z]{2})?$/u.test(value)
    ? value
    : fallback;
}

function selectedOrDefault(
  devices: readonly AudioDevice[],
  selectedId: string | null,
): AudioDevice | null {
  if (selectedId) {
    return devices.find((device) => device.id === selectedId) ?? null;
  }
  return devices.find((device) => device.isDefault) ?? devices[0] ?? null;
}

function invalid(
  code: SettingsValidationCode,
  channel: RealtimeChannel | null = null,
): SettingsValidation {
  return {
    valid: false,
    code,
    channel,
    feedbackRisk: false,
    feedbackConflict: null,
  };
}

function cloneDefaults(): WorkspaceSettings {
  return {
    microphone: { ...DEFAULT_WORKSPACE_SETTINGS.microphone },
    system: { ...DEFAULT_WORKSPACE_SETTINGS.system },
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function browserStorage(): StorageLike | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
