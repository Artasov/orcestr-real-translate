import { describe, expect, it } from "vitest";

import {
  DEFAULT_WORKSPACE_SETTINGS,
  WORKSPACE_SETTINGS_KEY,
  loadWorkspaceSettings,
  resolveDeviceId,
  saveWorkspaceSettings,
  systemFeedbackRisk,
  validateApiKeyDraft,
  validateWorkspaceStart,
  WINDOWS_PROCESS_LOOPBACK_ID,
} from "./settings";
import type { AudioDevices, WorkspaceSettings } from "./types";

const devices: AudioDevices = {
  inputs: [
    {
      id: "mic-default",
      name: "Microphone",
      isDefault: true,
      channels: 1,
      sampleRate: 48_000,
    },
  ],
  outputs: [
    {
      id: "speakers",
      name: "Speakers",
      isDefault: true,
      channels: 2,
      sampleRate: 48_000,
      excludesCurrentProcessAudio: true,
    },
    {
      id: "headphones",
      name: "Headphones",
      isDefault: false,
      channels: 2,
      sampleRate: 48_000,
    },
  ],
  systemSources: [
    {
      id: WINDOWS_PROCESS_LOOPBACK_ID,
      name: "All system audio",
      isDefault: true,
      channels: 2,
      sampleRate: 48_000,
    },
  ],
};

class MemoryStorage {
  values = new Map<string, string>();

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}

function configuredSettings(): WorkspaceSettings {
  return {
    microphone: {
      ...DEFAULT_WORKSPACE_SETTINGS.microphone,
      outputDeviceId: "headphones",
    },
    system: {
      ...DEFAULT_WORKSPACE_SETTINGS.system,
      enabled: true,
      inputDeviceId: WINDOWS_PROCESS_LOOPBACK_ID,
      outputDeviceId: "headphones",
    },
  };
}

describe("workspace settings", () => {
  it("falls back safely when persisted settings are malformed", () => {
    const storage = new MemoryStorage();
    storage.setItem(WORKSPACE_SETTINGS_KEY, "{broken");
    expect(loadWorkspaceSettings(storage)).toEqual(DEFAULT_WORKSPACE_SETTINGS);
  });

  it("persists only the normalized workspace schema and never extra secrets", () => {
    const storage = new MemoryStorage();
    const settings = {
      ...configuredSettings(),
      apiKey: "sk-this-must-never-be-persisted",
    } as WorkspaceSettings & { apiKey: string };

    expect(saveWorkspaceSettings(settings, storage)).toBe(true);
    const raw = storage.getItem(WORKSPACE_SETTINGS_KEY) ?? "";
    expect(raw).not.toContain("sk-this-must-never-be-persisted");
    expect(loadWorkspaceSettings(storage)).toEqual(configuredSettings());
  });

  it("allows translated speech on an output excluded from system capture", () => {
    const settings = configuredSettings();
    settings.system.outputDeviceId = "speakers";

    expect(systemFeedbackRisk(settings, devices)).toBe(false);
    expect(validateWorkspaceStart(settings, devices, true).valid).toBe(true);
  });

  it("does not report a loop for the process-excluded Windows source", () => {
    const settings = configuredSettings();
    settings.system.inputDeviceId = null;
    settings.system.outputDeviceId = null;
    expect(systemFeedbackRisk(settings, devices)).toBe(false);

    settings.system.mode = "transcribe";
    expect(systemFeedbackRisk(settings, devices)).toBe(false);
    expect(validateWorkspaceStart(settings, devices, true).valid).toBe(true);

    settings.system.mode = "translate";
    settings.system.playbackEnabled = false;
    expect(systemFeedbackRisk(settings, devices)).toBe(false);
    expect(validateWorkspaceStart(settings, devices, true).valid).toBe(true);
  });

  it("allows text-only translation without an audio output", () => {
    const settings = configuredSettings();
    settings.microphone.playbackEnabled = false;
    settings.system.enabled = false;

    expect(
      validateWorkspaceStart(settings, { ...devices, outputs: [] }, true).valid,
    ).toBe(true);
  });

  it("blocks Linux default-monitor playback only on the monitored output", () => {
    const linuxDevices: AudioDevices = {
      ...devices,
      systemSources: [
        {
          id: "linux-default-monitor:cpal:speakers",
          name: "Default system audio",
          isDefault: true,
          channels: 2,
          sampleRate: 48_000,
          monitoredOutputId: "speakers",
          excludesCurrentProcessAudio: false,
        },
      ],
    };
    const settings = configuredSettings();
    settings.system.inputDeviceId = linuxDevices.systemSources[0].id;
    settings.system.outputDeviceId = "speakers";
    expect(systemFeedbackRisk(settings, linuxDevices)).toBe(true);

    settings.system.outputDeviceId = "headphones";
    expect(systemFeedbackRisk(settings, linuxDevices)).toBe(false);
  });

  it("allows microphone translation on an output excluded from system capture", () => {
    const settings = configuredSettings();
    settings.microphone.outputDeviceId = "speakers";

    const validation = validateWorkspaceStart(settings, devices, true);
    expect(validation.valid).toBe(true);
    expect(validation.feedbackRisk).toBe(false);
  });

  it("resolves selected and default ids from one device inventory snapshot", () => {
    expect(resolveDeviceId(devices.outputs, null)).toBe("speakers");
    expect(resolveDeviceId(devices.outputs, "headphones")).toBe("headphones");
    expect(resolveDeviceId(devices.outputs, "missing")).toBeNull();
  });

  it("requires a key and at least one enabled channel", () => {
    const settings = configuredSettings();
    expect(validateWorkspaceStart(settings, devices, false).code).toBe(
      "api_key_required",
    );

    settings.microphone.enabled = false;
    settings.system.enabled = false;
    expect(validateWorkspaceStart(settings, devices, true).code).toBe(
      "channel_required",
    );
  });

  it("validates API key drafts without storing them", () => {
    expect(validateApiKeyDraft(" ")).toBeTruthy();
    expect(validateApiKeyDraft("sk short key")).toBe("whitespace");
    expect(validateApiKeyDraft("future-key-format")).toBeNull();
    expect(validateApiKeyDraft("sk-proj-1234567890abcdefghijk")).toBeNull();
  });
});
