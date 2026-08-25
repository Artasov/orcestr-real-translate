import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface UpdateSnapshot {
  available: boolean;
  currentVersion: string;
  version: string | null;
  notes: string | null;
}

export interface UpdateProgress {
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
}

export const nativeUpdate = {
  check: () => invoke<UpdateSnapshot>("check_app_update"),
  install: () => invoke<void>("install_app_update"),
  onAvailable: (
    handler: (snapshot: UpdateSnapshot) => void,
  ): Promise<() => void> =>
    listen<UpdateSnapshot>("update:available", ({ payload }) =>
      handler(payload),
    ),
  onProgress: (
    handler: (progress: UpdateProgress) => void,
  ): Promise<() => void> =>
    listen<UpdateProgress>("update:progress", ({ payload }) =>
      handler(payload),
    ),
  onError: (handler: (message: string) => void): Promise<() => void> =>
    listen<string>("update:error", ({ payload }) => handler(payload)),
};
