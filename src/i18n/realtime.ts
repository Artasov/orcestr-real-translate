import type {
  ApiKeyValidationCode,
  FeedbackConflict,
  SettingsValidation,
} from "../realtime/settings";
import type { AppText } from "./messages";
import { formatMessage } from "./messages";

export function settingsValidationMessage(
  validation: SettingsValidation,
  copy: AppText["validation"],
): string | null {
  switch (validation.code) {
    case "api_key_required":
      return copy.apiKeyRequired;
    case "channel_required":
      return copy.channelRequired;
    case "microphone_unavailable":
      return copy.microphoneUnavailable;
    case "system_unavailable":
      return copy.systemUnavailable;
    case "source_unavailable":
      return copy.sourceUnavailable;
    case "output_unavailable":
      return copy.outputUnavailable;
    case "selected_output_unavailable":
      return copy.selectedOutputUnavailable;
    case "language_required":
      return copy.languageRequired;
    case "feedback":
      return validation.feedbackConflict
        ? feedbackConflictMessage(validation.feedbackConflict, copy)
        : copy.realtimeFailed;
    default:
      return null;
  }
}

export function feedbackConflictMessage(
  conflict: FeedbackConflict,
  copy: AppText["validation"],
): string {
  const route =
    conflict.outputChannel === "microphone"
      ? copy.feedbackMicrophone
      : copy.feedbackSystem;
  return formatMessage(copy.feedback, {
    route,
    output: conflict.output.name,
  });
}

export function apiKeyValidationMessage(
  code: ApiKeyValidationCode,
  copy: AppText["validation"],
): string {
  switch (code) {
    case "empty":
      return copy.keyEmpty;
    case "whitespace":
      return copy.keyWhitespace;
    case "too_long":
      return copy.keyTooLong;
  }
}
