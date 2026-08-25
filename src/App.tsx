import {
  LoginForm,
  type AuthLegalConsentOptions,
  type OAuthButtonsOptions,
  type OAuthProviderButtonProps,
} from "@orcestr/auth-forms";
import type { OAuthProvider } from "@orcestr/auth-core";
import { useCurrentUser } from "@orcestr/auth-react";
import { isApiError } from "@orcestr/core";
import {
  Alert,
  Button,
  Card,
  Flex,
  IconButton,
  Spinner,
  Stack,
  Text,
  Tooltip,
} from "@orcestr/ui";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { FaGithub, FaYandex } from "react-icons/fa6";
import { FcGoogle } from "react-icons/fc";

import {
  authClient,
  isAllowedLegalDocumentUrl,
  nativeAuth,
  type OrcestrUser,
} from "./auth";
import { applyAuthSnapshotToCurrentUser } from "./auth-session";
import { localizedErrorMessage } from "./i18n/errors";
import { useAppI18n } from "./i18n/I18nProvider";
import { LanguageMenu } from "./i18n/LanguageMenu";
import { legalDocumentsLocale } from "./i18n/locales";
import { nativeUpdate, type UpdateSnapshot } from "./update";
import { TranslateWorkspace } from "./workspace/TranslateWorkspace";

function providerIcon(provider: OAuthProvider): ReactNode {
  switch (provider) {
    case "google":
      return <FcGoogle size={20} aria-hidden />;
    case "github":
      return <FaGithub size={20} aria-hidden />;
    case "yandex":
      return <FaYandex size={20} aria-hidden />;
  }
}

function OAuthProviderButton({
  provider,
  label,
  onClick,
  disabled,
}: OAuthProviderButtonProps) {
  return (
    <Tooltip content={label}>
      <IconButton
        type="button"
        v="soft"
        tone="neutral"
        size={3}
        round
        aria-label={label}
        disabled={disabled}
        testId={`auth-oauth-${provider}`}
        onClick={onClick}
      >
        {providerIcon(provider)}
      </IconButton>
    </Tooltip>
  );
}

function profileLabel(profile: OrcestrUser, fallback: string): string {
  for (const key of [
    "name",
    "first_name",
    "preferred_username",
    "username",
    "email",
  ]) {
    const value = profile[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return fallback;
}

function legalDocumentHref(document: {
  documentSlug: string;
  language: "en" | "ru";
}): string {
  const localePrefix = document.language === "ru" ? "/ru" : "";
  return `https://orcestr.com${localePrefix}/legal/${document.documentSlug}`;
}

export function App() {
  const { app, auth, format, locale } = useAppI18n();
  const queryClient = useQueryClient();
  const currentUser = useCurrentUser<OrcestrUser>();
  const logout = useMutation({
    mutationFn: () => authClient.logout(),
    onSuccess: () => {
      applyAuthSnapshotToCurrentUser(queryClient, {
        phase: "signedOut",
        profile: null,
        message: null,
      });
    },
  });
  const [oauthProvider, setOauthProvider] = useState<OAuthProvider | null>(
    null,
  );
  const [oauthActionPending, setOauthActionPending] = useState<
    "cancel" | "retry" | null
  >(null);
  const oauthActionInFlight = useRef(false);
  const [authTransportError, setAuthTransportError] = useState<string | null>(
    null,
  );
  const [availableUpdate, setAvailableUpdate] = useState<UpdateSnapshot | null>(
    null,
  );
  const [updatePercent, setUpdatePercent] = useState<number | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [isInstallingUpdate, setIsInstallingUpdate] = useState(false);
  const updateInstallInFlight = useRef(false);

  const methods = useQuery({
    queryKey: ["auth-methods", "native-browser-origin"],
    queryFn: () => authClient.methods(),
  });
  const legalDocuments = useQuery({
    queryKey: ["auth-legal-documents", locale],
    queryFn: () => nativeAuth.legalDocuments(legalDocumentsLocale(locale)),
  });

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const remember = (unlisten: () => void) => {
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    };

    void nativeAuth
      .onChanged((snapshot) => {
        if (disposed) return;
        applyAuthSnapshotToCurrentUser(queryClient, snapshot);
        if (snapshot.phase === "authenticated" && snapshot.profile) {
          setOauthProvider(null);
          setAuthTransportError(null);
        } else if (snapshot.phase === "signedOut") {
          setOauthProvider(null);
        } else if (snapshot.phase === "error") {
          setOauthProvider(null);
          setAuthTransportError(app.common.genericError);
        }
      })
      .then(remember);
    void nativeUpdate.onAvailable(setAvailableUpdate).then(remember);
    void nativeUpdate
      .onProgress((progress) => setUpdatePercent(progress.percent))
      .then(remember);
    void nativeUpdate
      .onError(() => setUpdateError(app.common.genericError))
      .then(remember);

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [app.common.genericError, queryClient]);

  useEffect(() => {
    const onClick = (event: MouseEvent) => {
      if (event.defaultPrevented || !(event.target instanceof Element)) return;
      const anchor = event.target.closest<HTMLAnchorElement>(
        "a[target='_blank'][href]",
      );
      if (!anchor || !isAllowedLegalDocumentUrl(anchor.href)) return;
      event.preventDefault();
      void nativeAuth.openLegalDocument(anchor.href).catch((error: unknown) => {
        setAuthTransportError(localizedErrorMessage(error, app.common, auth));
      });
    };
    document.addEventListener("click", onClick);
    return () => document.removeEventListener("click", onClick);
  }, [app.common, auth]);

  const beginOAuth = useCallback(
    async (provider: OAuthProvider) => {
      setAuthTransportError(null);
      setOauthProvider(provider);
      try {
        await nativeAuth.beginOAuth(provider);
      } catch (error) {
        if (
          !isApiError(error) ||
          error.code !== "oauth_authorization_in_progress"
        ) {
          setOauthProvider(null);
        }
        setAuthTransportError(localizedErrorMessage(error, app.common, auth));
      }
    },
    [app.common, auth],
  );

  const cancelOAuth = useCallback(async () => {
    if (oauthActionInFlight.current) return;
    oauthActionInFlight.current = true;
    setOauthActionPending("cancel");
    try {
      await nativeAuth.cancelOAuth();
      setOauthProvider(null);
      setAuthTransportError(null);
    } catch (error) {
      setAuthTransportError(localizedErrorMessage(error, app.common, auth));
    } finally {
      oauthActionInFlight.current = false;
      setOauthActionPending(null);
    }
  }, [app.common, auth]);

  const retryOAuth = useCallback(async () => {
    if (!oauthProvider || oauthActionInFlight.current) return;
    oauthActionInFlight.current = true;
    const provider = oauthProvider;
    setOauthActionPending("retry");
    setAuthTransportError(null);
    try {
      await nativeAuth.cancelOAuth();
      await nativeAuth.beginOAuth(provider);
      setOauthProvider(provider);
    } catch (error) {
      setOauthProvider(null);
      setAuthTransportError(localizedErrorMessage(error, app.common, auth));
    } finally {
      oauthActionInFlight.current = false;
      setOauthActionPending(null);
    }
  }, [app.common, auth, oauthProvider]);

  const oauthButtons = useMemo<OAuthButtonsOptions>(
    () => ({
      placement: "after-submit",
      direction: "row",
      justify: "center",
      gap: "2",
      buttonComponent: OAuthProviderButton,
      authorizeHandler: (provider) => beginOAuth(provider),
    }),
    [beginOAuth],
  );

  const legalConsent = useMemo<AuthLegalConsentOptions | undefined>(() => {
    if (!legalDocuments.data) return undefined;
    return {
      enabled: true,
      selectAllOnFirstDocumentCheck: true,
      storage: { key: "orcestr-real-translate:auth-legal-consent" },
      documents: legalDocuments.data.map((document) => ({
        id: document.documentSlug,
        title: document.title,
        version: document.version,
        href: legalDocumentHref(document),
        required: document.requiredForRegistration,
        acceptance: {
          document_slug: document.documentSlug,
          version: document.version,
          language: document.language,
        },
      })),
    };
  }, [legalDocuments.data]);

  const installUpdate = async () => {
    if (updateInstallInFlight.current) return;
    updateInstallInFlight.current = true;
    setIsInstallingUpdate(true);
    setUpdateError(null);
    setUpdatePercent(0);
    try {
      await nativeUpdate.install();
    } catch (error) {
      setUpdateError(localizedErrorMessage(error, app.common, auth));
      setUpdatePercent(null);
    } finally {
      updateInstallInFlight.current = false;
      setIsInstallingUpdate(false);
    }
  };

  const user = currentUser.data;
  const signedOutError =
    currentUser.error &&
    (!isApiError(currentUser.error) ||
      currentUser.error.code !== "not_authenticated")
      ? localizedErrorMessage(currentUser.error, app.common, auth)
      : null;

  return (
    <main className={`app-shell${user ? " is-authenticated" : ""}`}>
      <div className="square-grid" aria-hidden="true" />
      <div className="ambient ambient-violet" aria-hidden="true" />

      <section className={`center-stage${user ? " workspace-stage" : ""}`}>
        {currentUser.isLoading ? (
          <div className="loading-state" role="status" aria-live="polite">
            <Spinner size={3} />
            <Text className="loading-label" tone="muted">
              {app.auth.restoring}
            </Text>
          </div>
        ) : user ? (
          <TranslateWorkspace
            user={user}
            profileName={profileLabel(user, app.common.profileFallback)}
            signOutPending={logout.isPending}
            signOutError={
              logout.error
                ? localizedErrorMessage(logout.error, app.common, auth)
                : null
            }
            onSignOut={() => logout.mutate()}
          />
        ) : (
          <Card className="auth-card" v="surface" size={4}>
            <div className="auth-language-menu">
              <LanguageMenu />
            </div>
            <Stack g="4">
              <Stack g="2">
                <Text fs="12px" fw={760} tone="primary" className="eyebrow">
                  {app.auth.eyebrow}
                </Text>
                <Text as="h1" className="screen-title">
                  {app.auth.title}
                </Text>
                <Text tone="muted" className="lead">
                  {app.auth.lead}
                </Text>
              </Stack>

              {methods.isError ? (
                <Alert
                  tone="danger"
                  action={
                    <Button
                      type="button"
                      size={1}
                      v="surface"
                      loading={methods.isFetching}
                      onClick={() => void methods.refetch()}
                    >
                      {app.common.retry}
                    </Button>
                  }
                >
                  {app.auth.methodsError}
                </Alert>
              ) : null}
              {legalDocuments.isError ? (
                <Alert
                  tone="danger"
                  action={
                    <Button
                      type="button"
                      size={1}
                      v="surface"
                      loading={legalDocuments.isFetching}
                      onClick={() => void legalDocuments.refetch()}
                    >
                      {app.common.retry}
                    </Button>
                  }
                >
                  {app.auth.legalError}
                </Alert>
              ) : null}
              {signedOutError ? (
                <Alert tone="danger">{signedOutError}</Alert>
              ) : null}
              {authTransportError ? (
                <Alert tone="danger">{authTransportError}</Alert>
              ) : null}
              {oauthProvider ? (
                <Alert
                  tone="info"
                  title={app.auth.oauthTitle}
                  action={
                    <Flex g="1">
                      <Button
                        type="button"
                        size={1}
                        v="ghost"
                        loading={oauthActionPending === "cancel"}
                        disabled={oauthActionPending !== null}
                        onClick={() => void cancelOAuth()}
                      >
                        {app.common.cancel}
                      </Button>
                      <Button
                        type="button"
                        size={1}
                        v="surface"
                        tone="primary"
                        loading={oauthActionPending === "retry"}
                        disabled={oauthActionPending !== null}
                        onClick={() => void retryOAuth()}
                      >
                        {app.common.retry}
                      </Button>
                    </Flex>
                  }
                >
                  {format(app.auth.oauthBody, { provider: oauthProvider })}
                </Alert>
              ) : null}

              <div className="auth-form-wrap">
                <LoginForm<OrcestrUser>
                  methods={methods.data}
                  next="/oauth/authorize"
                  oauthButtons={oauthButtons}
                  oauthLegalConsent={false}
                  legalConsent={legalConsent}
                  disabled={
                    methods.isLoading ||
                    methods.isError ||
                    legalDocuments.isLoading ||
                    legalDocuments.isError ||
                    Boolean(oauthProvider) ||
                    oauthActionPending !== null
                  }
                  onSuccess={() => {
                    setAuthTransportError(null);
                    setOauthProvider(null);
                  }}
                />
              </div>

              <Text fs="11px" tone="muted" ta="center">
                {app.auth.secureNote}
              </Text>
            </Stack>
          </Card>
        )}
      </section>

      {availableUpdate?.available ? (
        <Card className="update-card" v="surface" size={3} aria-live="polite">
          <Flex a="c" g="3">
            <Stack g="1" className="update-copy">
              <Text fw={700}>
                {format(app.auth.updateReady, {
                  version:
                    availableUpdate.version ?? availableUpdate.currentVersion,
                })}
              </Text>
              <Text fs="12px" tone={updateError ? "danger" : "muted"}>
                {updateError ??
                  (updatePercent === null
                    ? app.auth.updateSigned
                    : format(app.auth.downloading, { percent: updatePercent }))}
              </Text>
            </Stack>
            <Button
              type="button"
              tone="primary"
              v="soft"
              onClick={installUpdate}
              disabled={isInstallingUpdate}
              loading={isInstallingUpdate}
            >
              {isInstallingUpdate ? app.auth.installing : app.auth.install}
            </Button>
          </Flex>
        </Card>
      ) : null}

      {!user ? (
        <footer>
          <Text fs="10px" tone="muted">
            {app.auth.privateByDesign}
          </Text>
          <span aria-hidden="true">•</span>
          <Text fs="10px" tone="muted">
            {app.auth.nativeSecureStorage}
          </Text>
        </footer>
      ) : null}
    </main>
  );
}
