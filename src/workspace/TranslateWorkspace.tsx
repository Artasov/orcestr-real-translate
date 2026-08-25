import { Button, Tabs } from "@orcestr/ui";
import { useCallback, useEffect, useMemo, useReducer, useState } from "react";
import { LuPlay, LuRadio, LuSettings, LuSquare } from "react-icons/lu";

import type { OrcestrUser } from "../auth";
import { localizedErrorMessage } from "../i18n/errors";
import { useAppI18n } from "../i18n/I18nProvider";
import { LanguageMenu } from "../i18n/LanguageMenu";
import { settingsValidationMessage } from "../i18n/realtime";
import { nativeRealtime } from "../realtime/native";
import {
  initialRealtimeSession,
  isChannelActive,
  realtimeSessionReducer,
} from "../realtime/session";
import {
  loadWorkspaceSettings,
  resolveDeviceId,
  saveWorkspaceSettings,
  validateWorkspaceStart,
} from "../realtime/settings";
import type {
  AudioDevices,
  ChannelSettings,
  RealtimeChannel,
  WorkspaceSettings,
} from "../realtime/types";
import { LiveView } from "./LiveView";
import { SettingsView } from "./SettingsView";

const EMPTY_DEVICES: AudioDevices = {
  inputs: [],
  outputs: [],
  systemSources: [],
};

const NO_PLAYBACK_PENDING: Record<RealtimeChannel, boolean> = {
  microphone: false,
  system: false,
};

interface TranslateWorkspaceProps {
  user: OrcestrUser;
  profileName: string;
  signOutPending: boolean;
  signOutError: string | null;
  onSignOut: () => void;
}

export function TranslateWorkspace({
  user,
  profileName,
  signOutPending,
  signOutError,
  onSignOut,
}: TranslateWorkspaceProps) {
  const { app, auth } = useAppI18n();
  const [activeTab, setActiveTab] = useState("live");
  const [settings, setSettings] = useState<WorkspaceSettings>(() =>
    loadWorkspaceSettings(),
  );
  const [devices, setDevices] = useState<AudioDevices>(EMPTY_DEVICES);
  const [devicesLoading, setDevicesLoading] = useState(true);
  const [devicesError, setDevicesError] = useState<string | null>(null);
  const [keyConfigured, setKeyConfigured] = useState(false);
  const [keyLoading, setKeyLoading] = useState(true);
  const [actionPending, setActionPending] = useState(false);
  const [playbackPending, setPlaybackPending] = useState(NO_PLAYBACK_PENDING);
  const [engineError, setEngineError] = useState<string | null>(null);
  const [session, dispatch] = useReducer(
    realtimeSessionReducer,
    undefined,
    initialRealtimeSession,
  );

  const running = useMemo(
    () =>
      isChannelActive(session.channels.microphone) ||
      isChannelActive(session.channels.system),
    [session.channels],
  );
  const startValidation = useMemo(
    () => validateWorkspaceStart(settings, devices, keyConfigured),
    [devices, keyConfigured, settings],
  );

  useEffect(() => {
    saveWorkspaceSettings(settings);
  }, [settings]);

  const refreshDevices = useCallback(async () => {
    setDevicesLoading(true);
    setDevicesError(null);
    try {
      setDevices(await nativeRealtime.listDevices());
    } catch (error) {
      setDevicesError(localizedErrorMessage(error, app.common, auth));
    } finally {
      setDevicesLoading(false);
    }
  }, [app.common, auth]);

  const refreshKeyStatus = useCallback(async () => {
    setKeyLoading(true);
    try {
      const status = await nativeRealtime.keyStatus();
      setKeyConfigured(status.configured);
    } catch (error) {
      setEngineError(localizedErrorMessage(error, app.common, auth));
    } finally {
      setKeyLoading(false);
    }
  }, [app.common, auth]);

  useEffect(() => {
    void refreshDevices();
    void refreshKeyStatus();
  }, [refreshDevices, refreshKeyStatus]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void nativeRealtime
      .onEvent((event) => {
        if (!disposed) dispatch({ type: "event", event });
      })
      .then((nextUnlisten) => {
        if (disposed) nextUnlisten();
        else unlisten = nextUnlisten;
      });
    return () => {
      disposed = true;
      unlisten?.();
      void nativeRealtime.stopAll().catch(() => undefined);
    };
  }, []);

  const changeChannel = useCallback(
    (channel: RealtimeChannel, patch: Partial<ChannelSettings>) => {
      if (running || actionPending) return;
      setSettings((current) => ({
        ...current,
        [channel]: { ...current[channel], ...patch },
      }));
      setEngineError(null);
    },
    [actionPending, running],
  );

  const startSession = useCallback(async () => {
    const validation = validateWorkspaceStart(settings, devices, keyConfigured);
    if (!validation.valid) {
      setEngineError(
        settingsValidationMessage(validation, app.validation) ??
          app.validation.realtimeFailed,
      );
      if (!keyConfigured || validation.feedbackRisk) setActiveTab("settings");
      return;
    }

    const channels = (["microphone", "system"] as const).filter(
      (channel) => settings[channel].enabled,
    );
    setActionPending(true);
    setEngineError(null);
    for (const channel of channels) dispatch({ type: "starting", channel });
    try {
      await Promise.all(
        channels.map((channel) => {
          const config = settings[channel];
          const sources =
            channel === "microphone" ? devices.inputs : devices.systemSources;
          return nativeRealtime.start({
            channel,
            mode: config.mode,
            playbackEnabled:
              config.mode === "translate" && config.playbackEnabled,
            inputDeviceId: resolveDeviceId(sources, config.inputDeviceId),
            outputDeviceId:
              config.mode === "translate"
                ? resolveDeviceId(devices.outputs, config.outputDeviceId)
                : null,
            targetLanguage:
              config.mode === "translate" ? config.targetLanguage : null,
          });
        }),
      );
    } catch (error) {
      await nativeRealtime.stopAll().catch(() => undefined);
      const message = localizedErrorMessage(error, app.common, auth);
      setEngineError(message);
      for (const channel of channels) {
        dispatch({
          type: "event",
          event: { channel, kind: "error", message },
        });
      }
    } finally {
      setActionPending(false);
    }
  }, [app.common, app.validation, auth, devices, keyConfigured, settings]);

  const changePlayback = useCallback(
    async (channel: RealtimeChannel, enabled: boolean) => {
      if (actionPending || playbackPending[channel]) return;
      const channelActive = isChannelActive(session.channels[channel]);
      if (!channelActive) {
        setSettings((current) => ({
          ...current,
          [channel]: { ...current[channel], playbackEnabled: enabled },
        }));
        setEngineError(null);
        return;
      }

      setPlaybackPending((current) => ({ ...current, [channel]: true }));
      setEngineError(null);
      try {
        await nativeRealtime.setPlaybackEnabled(channel, enabled);
        setSettings((current) => ({
          ...current,
          [channel]: { ...current[channel], playbackEnabled: enabled },
        }));
      } catch (error) {
        setEngineError(localizedErrorMessage(error, app.common, auth));
      } finally {
        setPlaybackPending((current) => ({ ...current, [channel]: false }));
      }
    },
    [actionPending, app.common, auth, playbackPending, session.channels],
  );

  const openSettings = useCallback(() => setActiveTab("settings"), []);
  const clearTranscript = useCallback(
    () => dispatch({ type: "clear-transcript" }),
    [],
  );
  const handlePlaybackChange = useCallback(
    (channel: RealtimeChannel, enabled: boolean) => {
      void changePlayback(channel, enabled);
    },
    [changePlayback],
  );

  const stopSession = useCallback(async () => {
    const activeChannels = (["microphone", "system"] as const).filter(
      (channel) => isChannelActive(session.channels[channel]),
    );
    setActionPending(true);
    setEngineError(null);
    for (const channel of activeChannels)
      dispatch({ type: "stopping", channel });
    try {
      await nativeRealtime.stopAll();
      for (const channel of activeChannels) {
        dispatch({
          type: "event",
          event: { channel, kind: "status", status: "idle" },
        });
      }
    } catch (error) {
      setEngineError(localizedErrorMessage(error, app.common, auth));
    } finally {
      setActionPending(false);
    }
  }, [app.common, auth, session.channels]);

  const toggleSession = useCallback(
    () => (running ? stopSession() : startSession()),
    [running, startSession, stopSession],
  );

  const saveKey = useCallback(async (apiKey: string) => {
    const status = await nativeRealtime.saveKey(apiKey);
    setKeyConfigured(status.configured);
    setEngineError(null);
  }, []);

  const deleteKey = useCallback(async () => {
    if (running) await nativeRealtime.stopAll();
    const status = await nativeRealtime.deleteKey();
    setKeyConfigured(status.configured);
    setEngineError(null);
  }, [running]);

  const signOut = useCallback(() => {
    void nativeRealtime.stopAll().finally(onSignOut);
  }, [onSignOut]);

  return (
    <div className="translate-workspace">
      <Tabs.Root
        value={activeTab}
        onValueChange={setActiveTab}
        className="workspace-tabs"
      >
        <div className="workspace-navigation">
          <div className="workspace-navigation-primary">
            <Tabs.List aria-label={app.workspace.sections}>
              <Tabs.Trigger value="live" icon={<LuRadio size={15} />}>
                {app.workspace.live}
              </Tabs.Trigger>
              <Tabs.Trigger value="settings" icon={<LuSettings size={15} />}>
                {app.workspace.settings}
              </Tabs.Trigger>
            </Tabs.List>
            <LanguageMenu />
          </div>
          <div className="workspace-navigation-actions">
            {activeTab === "live" || running ? (
              <Button
                type="button"
                tone={running ? "neutral" : "primary"}
                v={running ? "surface" : "soft"}
                size={2}
                leftIcon={
                  running ? <LuSquare size={14} /> : <LuPlay size={15} />
                }
                loading={actionPending}
                disabled={
                  devicesLoading ||
                  keyLoading ||
                  (!running && !startValidation.valid)
                }
                onClick={() => void toggleSession()}
                className="session-button"
                testId="session-toggle"
              >
                {running ? app.workspace.stop : app.workspace.start}
              </Button>
            ) : null}
          </div>
        </div>

        <div className="workspace-tab-panels">
          <Tabs.Content value="live" className="workspace-tab-panel">
            <LiveView
              settings={settings}
              devices={devices}
              session={session}
              keyConfigured={keyConfigured}
              actionPending={actionPending}
              playbackPending={playbackPending}
              error={engineError}
              onChannelChange={changeChannel}
              onPlaybackChange={handlePlaybackChange}
              onOpenSettings={openSettings}
              onClearTranscript={clearTranscript}
            />
          </Tabs.Content>
          <Tabs.Content value="settings" className="workspace-tab-panel">
            <SettingsView
              user={user}
              profileName={profileName}
              settings={settings}
              devices={devices}
              devicesLoading={devicesLoading}
              devicesError={devicesError}
              keyConfigured={keyConfigured}
              keyLoading={keyLoading}
              locked={running || actionPending}
              signOutPending={signOutPending}
              signOutError={signOutError}
              onChannelChange={changeChannel}
              onRefreshDevices={refreshDevices}
              onSaveKey={saveKey}
              onDeleteKey={deleteKey}
              onSignOut={signOut}
            />
          </Tabs.Content>
        </div>
      </Tabs.Root>
    </div>
  );
}
