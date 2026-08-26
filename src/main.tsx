import { AuthI18nProvider } from "@orcestr/auth-forms";
import { AuthProvider } from "@orcestr/auth-react";
import { OrcestrUiProvider, type OrcestrThemeOverrides } from "@orcestr/ui";
import "@orcestr/ui/styles.css";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { authClient } from "./auth";
import { installRendererDiagnostics } from "./diagnostics";
import { AppI18nProvider, useAppI18n } from "./i18n/I18nProvider";
import "./styles.css";

installRendererDiagnostics();

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: false, staleTime: 30_000 },
    mutations: { retry: false },
  },
});

const violetRole = {
  base: "#7c3aed",
  text: "#c4b5fd",
  surface: "rgb(124 58 237 / 16%)",
  border: "rgb(167 139 250 / 34%)",
  contrast: "#ffffff",
} as const;

const violetStatus = {
  color: "#a78bfa",
  text: "#ddd6fe",
  soft: "rgb(124 58 237 / 14%)",
  border: "rgb(167 139 250 / 32%)",
} as const;

const translateTheme: OrcestrThemeOverrides = {
  colors: {
    bg: "#000000",
    panel: "#060606",
    panelSoft: "#0d0d0d",
    section: "rgb(255 255 255 / 4%)",
    control: "rgb(255 255 255 / 3%)",
    controlHover: "rgb(255 255 255 / 7%)",
    text: "#f7f7fb",
    muted: "#aaa6b5",
    border: "rgb(255 255 255 / 12%)",
    borderStrong: "rgb(196 181 253 / 30%)",
    primary: violetRole,
    info: violetRole,
    success: violetRole,
    selected: "rgb(124 58 237 / 15%)",
    focusRing: "0 0 0 3px rgb(139 92 246 / 38%)",
    floating: "#070707",
    pad: "rgb(255 255 255 / 4%)",
    padHover: "rgb(255 255 255 / 7%)",
  },
  status: {
    primary: violetStatus,
    info: violetStatus,
    success: violetStatus,
  },
  shadows: {
    focus: "0 0 0 3px rgb(139 92 246 / 38%)",
  },
  components: {
    buttonRadius: "10px",
  },
};

const root = document.getElementById("root");

if (!root) {
  throw new Error("Application root is missing");
}

function LocalizedApplication() {
  const { auth, locale, ui } = useAppI18n();
  const sharedLocale = locale === "ru" ? "ru" : "en";

  return (
    <OrcestrUiProvider
      mode="dark"
      locale={sharedLocale}
      copy={ui}
      themeOverrides={translateTheme}
      className="translate-ui-root"
      toastPosition="top-right"
    >
      <AuthProvider client={authClient}>
        <AuthI18nProvider locale={sharedLocale} overrides={auth}>
          <App />
        </AuthI18nProvider>
      </AuthProvider>
    </OrcestrUiProvider>
  );
}

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <AppI18nProvider>
        <LocalizedApplication />
      </AppI18nProvider>
    </QueryClientProvider>
  </StrictMode>,
);
