import {
  Alert,
  Avatar,
  Badge,
  Button,
  Field,
  Flex,
  IconButton,
  ScrollArea,
  Section,
  Select,
  Stack,
  Text,
  TextField,
  Tooltip,
} from "@orcestr/ui";
import { memo, useState, type FormEvent, type ReactNode } from "react";
import {
  LuAudioLines,
  LuKeyRound,
  LuRefreshCw,
  LuShieldCheck,
} from "react-icons/lu";

import type { OrcestrUser } from "../auth";
import { localizedErrorMessage } from "../i18n/errors";
import { useAppI18n } from "../i18n/I18nProvider";
import {
  apiKeyValidationMessage,
  feedbackConflictMessage,
} from "../i18n/realtime";
import {
  resolveDeviceId,
  systemFeedbackConflict,
  validateApiKeyDraft,
} from "../realtime/settings";
import type {
  AudioDevice,
  AudioDevices,
  ChannelSettings,
  RealtimeChannel,
  WorkspaceSettings,
} from "../realtime/types";
import { InfoHint } from "./InfoHint";

interface SettingsViewProps {
  user: OrcestrUser;
  profileName: string;
  settings: WorkspaceSettings;
  devices: AudioDevices;
  devicesLoading: boolean;
  devicesError: string | null;
  keyConfigured: boolean;
  keyLoading: boolean;
  locked: boolean;
  signOutPending: boolean;
  signOutError: string | null;
  onChannelChange: (
    channel: RealtimeChannel,
    patch: Partial<ChannelSettings>,
  ) => void;
  onRefreshDevices: () => Promise<void>;
  onSaveKey: (apiKey: string) => Promise<void>;
  onDeleteKey: () => Promise<void>;
  onSignOut: () => void;
}

function SettingsViewComponent({
  user,
  profileName,
  settings,
  devices,
  devicesLoading,
  devicesError,
  keyConfigured,
  keyLoading,
  locked,
  signOutPending,
  signOutError,
  onChannelChange,
  onRefreshDevices,
  onSaveKey,
  onDeleteKey,
  onSignOut,
}: SettingsViewProps) {
  const { app, auth } = useAppI18n();
  const [keyDraft, setKeyDraft] = useState("");
  const [keyError, setKeyError] = useState<string | null>(null);
  const [keyPending, setKeyPending] = useState<"save" | "delete" | null>(null);
  const feedbackConflict = systemFeedbackConflict(settings, devices);

  const saveKey = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const validationCode = validateApiKeyDraft(keyDraft);
    if (validationCode) {
      setKeyError(apiKeyValidationMessage(validationCode, app.validation));
      return;
    }
    setKeyPending("save");
    setKeyError(null);
    try {
      await onSaveKey(keyDraft.trim());
      setKeyDraft("");
    } catch (error) {
      setKeyError(localizedErrorMessage(error, app.common, auth));
    } finally {
      setKeyPending(null);
    }
  };

  const deleteKey = async () => {
    setKeyPending("delete");
    setKeyError(null);
    try {
      await onDeleteKey();
      setKeyDraft("");
    } catch (error) {
      setKeyError(localizedErrorMessage(error, app.common, auth));
    } finally {
      setKeyPending(null);
    }
  };

  return (
    <ScrollArea
      className="settings-scroll"
      scrollbars="vertical"
      highlights
      highlightH={36}
      highlightColor="#000000"
    >
      <div className="settings-content">
        <Section className="settings-section" p={4} g={3} sectionOpacity={0.72}>
          <SettingsHeading
            icon={<LuKeyRound size={18} />}
            title={app.settings.openAiKey}
            help={app.settings.openAiKeyHelp}
            action={
              <Badge
                tone={keyConfigured ? "primary" : "neutral"}
                v="soft"
                size={1}
              >
                {keyLoading
                  ? app.settings.checking
                  : keyConfigured
                    ? app.settings.configured
                    : app.settings.notConfigured}
              </Badge>
            }
          />

          <form
            className="key-editor-row"
            onSubmit={(event) => void saveKey(event)}
          >
            <Field error={keyError} className="key-editor-field">
              <TextField
                type="password"
                aria-label={
                  keyConfigured
                    ? app.settings.replaceKeyAria
                    : app.settings.keyAria
                }
                autoComplete="off"
                spellCheck={false}
                value={keyDraft}
                placeholder={
                  keyConfigured
                    ? app.settings.replaceKeyPlaceholder
                    : app.settings.keyPlaceholder
                }
                disabled={keyPending !== null}
                onChange={(event) => {
                  setKeyDraft(event.target.value);
                  setKeyError(null);
                }}
                leftSlot={<LuShieldCheck size={15} aria-hidden="true" />}
              />
            </Field>
            <Button
              type="submit"
              tone="primary"
              v="soft"
              size={2}
              loading={keyPending === "save"}
              disabled={!keyDraft.trim() || keyPending !== null}
            >
              {keyConfigured ? app.settings.replaceKey : app.settings.saveKey}
            </Button>
            {keyConfigured ? (
              <Button
                type="button"
                tone="danger"
                v="ghost"
                size={2}
                loading={keyPending === "delete"}
                disabled={keyPending !== null}
                onClick={() => void deleteKey()}
              >
                {app.settings.removeKey}
              </Button>
            ) : null}
          </form>
        </Section>

        <Section className="settings-section" p={4} g={3} sectionOpacity={0.72}>
          <SettingsHeading
            icon={<LuAudioLines size={18} />}
            title={app.settings.audioRouting}
            help={app.settings.audioRoutingHelp}
            action={
              <Tooltip content={app.settings.refreshDevices}>
                <IconButton
                  icon={<LuRefreshCw size={16} />}
                  aria-label={app.settings.refreshDevices}
                  v="ghost"
                  size={2}
                  loading={devicesLoading}
                  disabled={locked}
                  onClick={() => void onRefreshDevices()}
                />
              </Tooltip>
            }
          />

          {locked ? (
            <Alert tone="info" v="soft">
              {app.settings.stopBeforeRouting}
            </Alert>
          ) : null}
          {devicesError ? (
            <Alert
              tone="danger"
              v="soft"
              action={
                <Button
                  size={1}
                  v="surface"
                  loading={devicesLoading}
                  onClick={() => void onRefreshDevices()}
                >
                  {app.common.retry}
                </Button>
              }
            >
              {devicesError}
            </Alert>
          ) : null}
          {feedbackConflict ? (
            <Alert tone="warning" v="soft" title={app.settings.feedbackBlocked}>
              {feedbackConflictMessage(feedbackConflict, app.validation)}
            </Alert>
          ) : null}

          <div className="routing-groups">
            <RoutingGroup
              title={app.channel.microphone}
              sourceLabel={app.settings.microphoneInput}
              outputLabel={app.settings.translatedOutput}
              sources={devices.inputs}
              outputs={devices.outputs}
              settings={settings.microphone}
              disabled={locked || devicesLoading}
              onChange={(patch) => onChannelChange("microphone", patch)}
            />
            <RoutingGroup
              title={app.channel.system}
              sourceLabel={app.settings.systemCapture}
              outputLabel={app.settings.translatedOutput}
              sources={devices.systemSources}
              outputs={devices.outputs}
              settings={settings.system}
              disabled={locked || devicesLoading}
              onChange={(patch) => onChannelChange("system", patch)}
            />
          </div>
        </Section>

        <Section
          className="settings-section account-section"
          p={3}
          g={2}
          sectionOpacity={0.72}
        >
          {signOutError ? <Alert tone="danger">{signOutError}</Alert> : null}
          <Flex a="c" g={3}>
            <Avatar size={4} fallback={profileName.slice(0, 1).toUpperCase()} />
            <Stack g={0} className="account-copy">
              <Text fw={700}>{profileName}</Text>
              <Text fs="12px" tone="muted">
                {user.email ?? user.username}
              </Text>
            </Stack>
            <Button
              type="button"
              v="surface"
              tone="neutral"
              size={2}
              loading={signOutPending}
              onClick={onSignOut}
              className="sign-out-button"
            >
              {app.settings.signOut}
            </Button>
          </Flex>
        </Section>
      </div>
    </ScrollArea>
  );
}

export const SettingsView = memo(SettingsViewComponent);

function SettingsHeading({
  icon,
  title,
  help,
  action,
}: {
  icon: ReactNode;
  title: string;
  help: string;
  action?: ReactNode;
}) {
  const { app, format } = useAppI18n();
  return (
    <Flex a="c" g={2} className="settings-heading">
      <span className="settings-heading-icon" aria-hidden="true">
        {icon}
      </span>
      <Flex a="c" g={1} className="settings-heading-copy">
        <Text fw={760}>{title}</Text>
        <InfoHint label={format(app.common.about, { title })} content={help} />
      </Flex>
      {action ? <div className="settings-heading-action">{action}</div> : null}
    </Flex>
  );
}

function RoutingGroup({
  title,
  sourceLabel,
  outputLabel,
  sources,
  outputs,
  settings,
  disabled,
  onChange,
}: {
  title: string;
  sourceLabel: string;
  outputLabel: string;
  sources: readonly AudioDevice[];
  outputs: readonly AudioDevice[];
  settings: ChannelSettings;
  disabled: boolean;
  onChange: (patch: Partial<ChannelSettings>) => void;
}) {
  const { app, format } = useAppI18n();
  return (
    <div className="routing-group">
      <Flex a="c" g={2} className="routing-group-title">
        <Text fw={700} fs="13px">
          {title}
        </Text>
      </Flex>
      <div className="routing-selects">
        <Select
          items={deviceItems(sources, app.settings.defaultSuffix)}
          value={settings.inputDeviceId ?? resolveDeviceId(sources, null)}
          selectedFallbackLabel={
            settings.inputDeviceId ? app.settings.unavailableDevice : undefined
          }
          placeholder={defaultDeviceLabel(
            sources,
            app.settings.systemDefault,
            app.settings.defaultDevice,
            format,
          )}
          floatingLabel={sourceLabel}
          clearable={Boolean(settings.inputDeviceId)}
          clearLabel={app.settings.useSystemDefault}
          size={2}
          disabled={disabled}
          onValueChange={(inputDeviceId) => onChange({ inputDeviceId })}
        />
        <Select
          items={deviceItems(outputs, app.settings.defaultSuffix)}
          value={settings.outputDeviceId ?? resolveDeviceId(outputs, null)}
          selectedFallbackLabel={
            settings.outputDeviceId ? app.settings.unavailableDevice : undefined
          }
          placeholder={defaultDeviceLabel(
            outputs,
            app.settings.systemDefault,
            app.settings.defaultDevice,
            format,
          )}
          floatingLabel={outputLabel}
          clearable={Boolean(settings.outputDeviceId)}
          clearLabel={app.settings.useSystemDefault}
          size={2}
          disabled={disabled || settings.mode === "transcribe"}
          onValueChange={(outputDeviceId) => onChange({ outputDeviceId })}
        />
      </div>
    </div>
  );
}

function deviceItems(devices: readonly AudioDevice[], defaultLabel: string) {
  return devices.map((device) => ({
    value: device.id,
    label: device.isDefault ? `${device.name} · ${defaultLabel}` : device.name,
    searchText: device.name,
  }));
}

function defaultDeviceLabel(
  devices: readonly AudioDevice[],
  fallback: string,
  template: string,
  format: (template: string, params: Record<string, string | number>) => string,
): string {
  const device = devices.find((candidate) => candidate.isDefault) ?? devices[0];
  return device ? format(template, { device: device.name }) : fallback;
}
