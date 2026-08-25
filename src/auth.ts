import type {
  AuthClientContract,
  AuthClientRoutes,
  AuthMethods,
  AuthUser,
  OAuthProvider,
} from "@orcestr/auth-core";
import { ApiError, type ApiFieldError, type ErrorParams } from "@orcestr/core";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type OrcestrUser = AuthUser & {
  first_name?: string | null;
  last_name?: string | null;
  avatar_url?: string | null;
};

export type AuthPhase =
  | "bootstrapping"
  | "signedOut"
  | "authorizing"
  | "authenticated"
  | "error";

export interface AuthSnapshot {
  phase: AuthPhase;
  profile: OrcestrUser | null;
  message: string | null;
}

export interface NativeLegalDocument {
  documentSlug: string;
  title: string;
  version: string;
  language: "en" | "ru";
  requiredForRegistration: boolean;
}

type NativeApiError = {
  status: number;
  code: string;
  message: string;
  fields?: readonly ApiFieldError[];
  params?: ErrorParams;
  requestId?: string;
};

type LegalAcceptance = {
  documentSlug: string;
  version: string;
  language: "en" | "ru";
};

const routes: AuthClientRoutes = {
  login: "native://auth/login",
  register: "native://auth/register",
  methods: "native://auth/methods",
  me: "native://auth/me",
  refresh: "native://auth/refresh",
  logout: "native://auth/logout",
  passwordResetRequest: "native://auth/password-reset/request",
  passwordResetConfirm: "native://auth/password-reset/confirm",
  emailVerificationCode: "native://auth/email/verification-code",
  emailConfirm: "native://auth/email/confirm",
  oauthCallback: (provider) => `native://auth/oauth/${provider}/callback`,
};

export class TauriAuthClient implements AuthClientContract<OrcestrUser> {
  readonly routes = routes;

  methods(origin?: string): Promise<AuthMethods> {
    if (origin !== undefined && origin !== "https://orcestr.com") {
      return Promise.reject(
        unsupported(
          "oauth_origin_not_allowed",
          "The authentication origin is not allowed.",
        ),
      );
    }
    return invokeAuth<AuthMethods>("auth_methods");
  }

  me(): Promise<OrcestrUser> {
    return invokeAuth<OrcestrUser>("auth_me");
  }

  async login(
    username: string,
    password: string,
    extraPayload?: Record<string, unknown>,
  ): Promise<{ user: OrcestrUser }> {
    const acceptedLegalDocuments = legalAcceptances(extraPayload);
    const user = await invokeAuth<OrcestrUser>("auth_login", {
      request: { username, password, acceptedLegalDocuments },
    });
    return { user };
  }

  register(_payload: Record<string, unknown>): Promise<{ user: OrcestrUser }> {
    return Promise.reject(
      unsupported(
        "native_registration_unavailable",
        "Account creation is not available in this desktop build.",
      ),
    );
  }

  async refresh(): Promise<{ user?: OrcestrUser }> {
    return { user: await invokeAuth<OrcestrUser>("auth_refresh") };
  }

  logout(): Promise<void> {
    return invokeAuth<void>("auth_logout");
  }

  requestPasswordReset(email: string): Promise<void> {
    return invokeAuth<void>("auth_password_reset_request", { email });
  }

  confirmPasswordReset(payload: {
    email: string;
    code: string;
    password: string;
  }): Promise<void> {
    return invokeAuth<void>("auth_password_reset_confirm", {
      request: payload,
    });
  }

  sendVerificationCode(): Promise<{ sent: boolean }> {
    return Promise.reject(
      unsupported(
        "native_email_verification_unavailable",
        "Email verification is not available from this screen.",
      ),
    );
  }

  confirmEmail(_code: string): Promise<OrcestrUser> {
    return Promise.reject(
      unsupported(
        "native_email_verification_unavailable",
        "Email verification is not available from this screen.",
      ),
    );
  }

  oauthCallback(
    _provider: OAuthProvider,
    _payload: Record<string, unknown>,
  ): Promise<{ user: OrcestrUser }> {
    return Promise.reject(
      unsupported(
        "native_oauth_callback_owned_by_tauri",
        "OAuth callbacks are handled by the secure desktop transport.",
      ),
    );
  }
}

export const authClient = new TauriAuthClient();

export const nativeAuth = {
  status: () => invokeAuth<AuthSnapshot>("auth_status"),
  bootstrap: () => invokeAuth<AuthSnapshot>("auth_bootstrap"),
  beginOAuth: (provider: OAuthProvider) =>
    invokeAuth<AuthSnapshot>("auth_begin_oauth", { provider }),
  cancelOAuth: () => invokeAuth<AuthSnapshot>("auth_cancel_oauth"),
  legalDocuments: (language: "en" | "ru") =>
    invokeAuth<NativeLegalDocument[]>("auth_legal_documents", { language }),
  openLegalDocument: (url: string) =>
    invokeAuth<void>("auth_open_legal_document", { url }),
  onChanged: (handler: (snapshot: AuthSnapshot) => void): Promise<UnlistenFn> =>
    listen<AuthSnapshot>("auth:changed", ({ payload }) => handler(payload)),
};

export function isAllowedLegalDocumentUrl(value: string): boolean {
  try {
    const url = new URL(value);
    if (
      url.protocol !== "https:" ||
      url.hostname !== "orcestr.com" ||
      url.port ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) {
      return false;
    }
    const parts = url.pathname.split("/").filter(Boolean);
    const slug =
      parts.length === 2 && parts[0] === "legal"
        ? parts[1]
        : parts.length === 3 && parts[0] === "ru" && parts[1] === "legal"
          ? parts[2]
          : null;
    return Boolean(slug && /^[a-z0-9-]{1,128}$/u.test(slug));
  } catch {
    return false;
  }
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  return "Something went wrong. Please try again.";
}

async function invokeAuth<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw asApiError(error);
  }
}

function asApiError(error: unknown): ApiError {
  if (error instanceof ApiError) return error;
  if (isNativeApiError(error)) {
    return new ApiError(error.status, {
      code: error.code,
      message: error.message,
      fields: error.fields ?? [],
      params: error.params,
      request_id: error.requestId,
    });
  }
  return new ApiError(0, {
    code: "network_error",
    message: errorMessage(error),
  });
}

function isNativeApiError(value: unknown): value is NativeApiError {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<NativeApiError>;
  return (
    typeof candidate.status === "number" &&
    Number.isInteger(candidate.status) &&
    candidate.status >= 0 &&
    candidate.status <= 599 &&
    validErrorCode(candidate.code) &&
    validErrorMessage(candidate.message) &&
    (candidate.params === undefined || isNativeErrorParams(candidate.params)) &&
    (candidate.requestId === undefined ||
      (typeof candidate.requestId === "string" &&
        candidate.requestId.length > 0 &&
        candidate.requestId.length <= 256 &&
        !containsUnsafeControl(candidate.requestId))) &&
    (candidate.fields === undefined ||
      (Array.isArray(candidate.fields) &&
        candidate.fields.length <= 64 &&
        candidate.fields.every(isNativeApiFieldError)))
  );
}

function isNativeApiFieldError(value: unknown): value is ApiFieldError {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<ApiFieldError>;
  return (
    Array.isArray(candidate.path) &&
    candidate.path.length <= 16 &&
    candidate.path.every(
      (part) =>
        (typeof part === "string" &&
          part.length > 0 &&
          part.length <= 128 &&
          !containsUnsafeControl(part)) ||
        (typeof part === "number" && Number.isSafeInteger(part)),
    ) &&
    validErrorCode(candidate.code) &&
    (candidate.message === undefined || validErrorMessage(candidate.message)) &&
    (candidate.params === undefined || isNativeErrorParams(candidate.params))
  );
}

function isNativeErrorParams(value: unknown): value is ErrorParams {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const entries = Object.entries(value);
  return (
    entries.length <= 32 &&
    entries.every(([key, item]) => {
      if (!key || key.length > 128 || containsUnsafeControl(key)) return false;
      return (
        item === null ||
        typeof item === "boolean" ||
        (typeof item === "number" &&
          Number.isFinite(item) &&
          Math.abs(item) <= Number.MAX_SAFE_INTEGER) ||
        (typeof item === "string" &&
          item.length <= 512 &&
          !containsUnsafeControl(item))
      );
    })
  );
}

function validErrorCode(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_.:-]{1,128}$/u.test(value);
}

function validErrorMessage(value: unknown): value is string {
  return (
    typeof value === "string" &&
    Boolean(value.trim()) &&
    value.length <= 2_048 &&
    !containsUnsafeControl(value, true)
  );
}

function containsUnsafeControl(
  value: string,
  allowWhitespace = false,
): boolean {
  for (const character of value) {
    const code = character.codePointAt(0) ?? 0;
    if (code < 0x20 || code === 0x7f) {
      if (
        allowWhitespace &&
        (character === "\n" || character === "\r" || character === "\t")
      ) {
        continue;
      }
      return true;
    }
  }
  return false;
}

function unsupported(code: string, message: string): ApiError {
  return new ApiError(501, { code, message });
}

function legalAcceptances(
  extraPayload?: Record<string, unknown>,
): LegalAcceptance[] {
  if (!extraPayload) return [];
  const unexpected = Object.keys(extraPayload).filter(
    (key) => key !== "accepted_legal_documents",
  );
  if (unexpected.length) {
    throw new ApiError(400, {
      code: "invalid_native_request",
      message: "The authentication payload contains unsupported fields.",
    });
  }
  const raw = extraPayload.accepted_legal_documents;
  if (raw === undefined) return [];
  if (!Array.isArray(raw) || raw.length > 16) {
    throw new ApiError(400, {
      code: "invalid_native_request",
      message: "The legal consent payload is invalid.",
    });
  }
  return raw.map((item) => {
    if (!item || typeof item !== "object") {
      throw invalidLegalPayload();
    }
    const acceptance = item as Record<string, unknown>;
    const keys = Object.keys(acceptance).sort();
    if (keys.join(",") !== "document_slug,language,version") {
      throw invalidLegalPayload();
    }
    const documentSlug = acceptance.document_slug;
    const version = acceptance.version;
    const language = acceptance.language;
    if (
      typeof documentSlug !== "string" ||
      !/^[a-z0-9-]{1,128}$/u.test(documentSlug) ||
      typeof version !== "string" ||
      !version.trim() ||
      version.length > 64 ||
      (language !== "en" && language !== "ru")
    ) {
      throw invalidLegalPayload();
    }
    return { documentSlug, version, language };
  });
}

function invalidLegalPayload(): ApiError {
  return new ApiError(400, {
    code: "invalid_native_request",
    message: "The legal consent payload is invalid.",
  });
}
