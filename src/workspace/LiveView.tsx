import { Alert, Button, Text } from "@orcestr/ui";
import { memo } from "react";

import { useAppI18n } from "../i18n/I18nProvider";
import {
  feedbackConflictMessage,
  settingsValidationMessage,
} from "../i18n/realtime";
import {
  isChannelActive,
  type RealtimeSessionState,
} from "../realtime/session";
import {
  systemFeedbackConflict,
  validateWorkspaceStart,
} from "../realtime/settings";
import type {
  AudioDevices,
  ChannelSettings,
  RealtimeChannel,
  WorkspaceSettings,
} from "../realtime/types";
import { ChannelPanel } from "./ChannelPanel";
import { TranscriptPanel } from "./TranscriptPanel";

interface LiveViewProps {
  settings: WorkspaceSettings;
  devices: AudioDevices;
  session: RealtimeSessionState;
  keyConfigured: boolean;
  actionPending: boolean;
  playbackPending: Record<RealtimeChannel, boolean>;
  error: string | null;
  onChannelChange: (
    channel: RealtimeChannel,
    patch: Partial<ChannelSettings>,
  ) => void;
  onPlaybackChange: (channel: RealtimeChannel, enabled: boolean) => void;
  onPlaybackVolumeChange: (
    channel: RealtimeChannel,
    volumeDb: number,
  ) => void;
  onOpenSettings: () => void;
  onClearTranscript: () => void;
}

function LiveViewComponent({
  settings,
  devices,
  session,
  keyConfigured,
  actionPending,
  playbackPending,
  error,
  onChannelChange,
  onPlaybackChange,
  onPlaybackVolumeChange,
  onOpenSettings,
  onClearTranscript,
}: LiveViewProps) {
  const { app } = useAppI18n();
  const running =
    isChannelActive(session.channels.microphone) ||
    isChannelActive(session.channels.system);
  const validation = validateWorkspaceStart(settings, devices, keyConfigured);
  const feedbackConflict = systemFeedbackConflict(settings, devices);
  return (
    <div className="live-view">
      <div className="live-notices">
        {!running && !validation.valid ? (
          <div
            className="live-setup-notice"
            data-tone={
              validation.feedbackRisk || keyConfigured ? "warning" : "info"
            }
            role="status"
          >
            <Text fs="12px">
              {keyConfigured
                ? settingsValidationMessage(validation, app.validation)
                : app.live.openAiKeyRequired}
            </Text>
            <Button type="button" size={1} v="ghost" onClick={onOpenSettings}>
              {app.common.settings}
            </Button>
          </div>
        ) : null}
        {error ? (
          <Alert className="live-alert" tone="danger" v="soft">
            {error}
          </Alert>
        ) : null}
        {feedbackConflict && running ? (
          <Alert className="live-alert" tone="warning" v="soft">
            {feedbackConflictMessage(feedbackConflict, app.validation)}
          </Alert>
        ) : null}
      </div>

      <div className="channel-grid">
        <ChannelPanel
          channel="microphone"
          settings={settings.microphone}
          runtime={session.channels.microphone}
          locked={running || actionPending}
          playbackLocked={actionPending}
          playbackPending={playbackPending.microphone}
          onChange={(patch) => onChannelChange("microphone", patch)}
          onPlaybackChange={(enabled) =>
            onPlaybackChange("microphone", enabled)
          }
          onPlaybackVolumeChange={(volumeDb) =>
            onPlaybackVolumeChange("microphone", volumeDb)
          }
        />
        <ChannelPanel
          channel="system"
          settings={settings.system}
          runtime={session.channels.system}
          locked={running || actionPending}
          playbackLocked={actionPending}
          playbackPending={playbackPending.system}
          onChange={(patch) => onChannelChange("system", patch)}
          onPlaybackChange={(enabled) => onPlaybackChange("system", enabled)}
          onPlaybackVolumeChange={(volumeDb) =>
            onPlaybackVolumeChange("system", volumeDb)
          }
        />
      </div>

      <TranscriptPanel
        entries={session.transcript}
        onClear={onClearTranscript}
      />
    </div>
  );
}

export const LiveView = memo(LiveViewComponent);
