import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import {
  appLocaleDirection,
  detectAppLocale,
  persistAppLocale,
  type AppLocale,
} from "./locales";
import { formatMessage, localeMessages, type LocaleMessages } from "./messages";

interface AppI18nContextValue extends LocaleMessages {
  locale: AppLocale;
  direction: "ltr" | "rtl";
  setLocale: (locale: AppLocale) => void;
  format: typeof formatMessage;
}

const AppI18nContext = createContext<AppI18nContextValue | null>(null);

export function AppI18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<AppLocale>(() => detectAppLocale());

  const setLocale = useCallback((nextLocale: AppLocale) => {
    persistAppLocale(nextLocale);
    setLocaleState(nextLocale);
  }, []);

  const direction = appLocaleDirection(locale);

  useEffect(() => {
    document.documentElement.lang = locale;
    document.documentElement.dir = direction;
  }, [direction, locale]);

  const value = useMemo<AppI18nContextValue>(
    () => ({
      ...localeMessages[locale],
      locale,
      direction,
      setLocale,
      format: formatMessage,
    }),
    [direction, locale, setLocale],
  );

  return (
    <AppI18nContext.Provider value={value}>{children}</AppI18nContext.Provider>
  );
}

export function useAppI18n(): AppI18nContextValue {
  const value = useContext(AppI18nContext);
  if (!value) {
    throw new Error("useAppI18n must be used inside AppI18nProvider");
  }
  return value;
}
