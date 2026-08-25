import {
  Badge,
  Flex,
  IconButton,
  Section,
  SegmentedControl,
  Select,
  Switch,
  Text,
  Tooltip,
} from "@orcestr/ui";
import { LuMic, LuMonitorUp, LuVolume2, LuVolumeX } from "react-icons/lu";

import { useAppI18n } from "../i18n/I18nProvider";
import type { AppText } from "../i18n/messages";
import type { ChannelRuntime } from "../realtime/session";
import type {
  ChannelSettings,
  RealtimeChannel,
  RealtimeMode,
} from "../realtime/types";
import { languageItems } from "./constants";
import { InfoHint } from "./InfoHint";

interface ChannelPanelProps {
  channel: RealtimeChannel;
  settings: ChannelSettings;
  runtime: ChannelRuntime;
  locked: boolean;
  playbackLocked: boolean;
  playbackPending: boolean;
  onChange: (patch: Partial<ChannelSettings>) => void;
  onPlaybackChange: (enabled: boolean) => void;
}

export function ChannelPanel({
  channel,
  settings,
  runtime,
  locked,
  playbackLocked,
  playbackPending,
  onChange,
  onPlaybackChange,
}: ChannelPanelProps) {
  const { app, format, locale } = useAppI18n();
  const microphone = channel === "microphone";
  const disabled = !settings.enabled;
  const status = statusPresentation(runtime.status, app.channel);
  const playbackOn = settings.mode === "translate" && settings.playbackEnabled;

  return (
    <Section
      className={`channel-panel${disabled ? " is-disabled" : ""}`}
      p={3}
      g={3}
      sectionOpacity={0.78}
      testId={`channel-${channel}`}
    >
      <Flex a="c" g={2} className="channel-heading">
        <span className="channel-icon" aria-hidden="true">
          {microphone ? <LuMic size={18} /> : <LuMonitorUp size={18} />}
        </span>
        <Flex a="c" g={1} className="channel-title-copy">
          <Text fw={760}>
            {microphone ? app.channel.microphone : app.channel.system}
          </Text>
          <InfoHint
            label={
              microphone ? app.channel.microphoneAbout : app.channel.systemAbout
            }
            content={
              microphone ? app.channel.microphoneHelp : app.channel.systemHelp
            }
          />
        </Flex>
        {status ? (
          <Badge
            tone={status.tone}
            v="soft"
            size={1}
            className="channel-status"
          >
            {status.label}
          </Badge>
        ) : null}
        <Switch
          checked={settings.enabled}
          disabled={locked}
          aria-label={
            microphone ? app.channel.microphoneAria : app.channel.systemAria
          }
          onCheckedChange={(enabled) => onChange({ enabled })}
        />
      </Flex>

      <div
        className="audio-level"
        aria-label={format(app.channel.inputLevel, {
          percent: Math.round(runtime.level * 100),
        })}
      >
        <span style={{ transform: `scaleX(${runtime.level})` }} />
      </div>

      <div className="channel-controls-row">
        <SegmentedControl
          value={settings.mode}
          onValueChange={(value) => onChange({ mode: value as RealtimeMode })}
          items={[
            {
              value: "transcribe",
              label: app.channel.transcribe,
              disabled: locked,
            },
            {
              value: "translate",
              label: app.channel.translate,
              disabled: locked,
            },
          ]}
          size={2}
          className="channel-mode"
          testId={`${channel}-mode`}
        />
        <Select
          items={languageItems(locale)}
          value={settings.targetLanguage}
          onValueChange={(targetLanguage) => {
            if (targetLanguage) onChange({ targetLanguage });
          }}
          floatingLabel={app.channel.language}
          size={2}
          disabled={locked || settings.mode === "transcribe"}
          testId={`${channel}-language`}
        />
        <Tooltip
          content={
            settings.mode === "transcribe"
              ? app.channel.playbackTranslateOnly
              : playbackOn
                ? app.channel.mutePlayback
                : app.channel.playPlayback
          }
        >
          <IconButton
            icon={
              playbackOn ? <LuVolume2 size={16} /> : <LuVolumeX size={16} />
            }
            aria-label={
              playbackOn
                ? app.channel.disablePlayback
                : app.channel.enablePlayback
            }
            aria-pressed={playbackOn}
            tone={playbackOn ? "primary" : "neutral"}
            v={playbackOn ? "soft" : "ghost"}
            size={2}
            className="channel-playback-toggle"
            testId={`${channel}-playback`}
            loading={playbackPending}
            disabled={
              disabled ||
              settings.mode === "transcribe" ||
              playbackLocked ||
              playbackPending
            }
            onClick={() => onPlaybackChange(!playbackOn)}
          />
        </Tooltip>
      </div>

      {runtime.error ? (
        <Text fs="11px" tone="danger" role="alert" className="channel-error">
          {app.validation.realtimeFailed}
        </Text>
      ) : null}
    </Section>
  );
}

function statusPresentation(
  status: ChannelRuntime["status"],
  copy: AppText["channel"],
): {
  label: string;
  tone: "neutral" | "info" | "danger";
} | null {
  switch (status) {
    case "starting":
      return { label: copy.starting, tone: "info" };
    case "listening":
      return { label: copy.live, tone: "info" };
    case "stopping":
      return { label: copy.stopping, tone: "neutral" };
    case "error":
      return { label: copy.error, tone: "danger" };
    default:
      return null;
  }
}
