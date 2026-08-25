import { isAuthErrorCode } from "@orcestr/auth-core";
import type { AuthMessages } from "@orcestr/auth-forms";
import { isApiError } from "@orcestr/core";

export function localizedErrorMessage(
  error: unknown,
  copy: { genericError: string; networkError: string },
  auth: AuthMessages,
): string {
  if (isApiError(error)) {
    if (isAuthErrorCode(error.code)) return auth.errors[error.code];
    if (error.code === "network_error" || error.status === 0) {
      return copy.networkError;
    }
  }
  return copy.genericError;
}
