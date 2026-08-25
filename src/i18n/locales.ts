export const APP_LOCALES = [
  "en",
  "ru",
  "ka",
  "kk",
  "uz",
  "tr",
  "zh",
  "hy",
  "fr",
  "pl",
  "ar",
] as const;

export type AppLocale = (typeof APP_LOCALES)[number];

export const APP_LOCALE_STORAGE_KEY = "orcestr-real-translate:locale";

export const APP_LOCALE_OPTIONS: ReadonlyArray<{
  value: AppLocale;
  label: string;
}> = [
  { value: "en", label: "English" },
  { value: "ru", label: "Русский" },
  { value: "ka", label: "ქართული" },
  { value: "kk", label: "Қазақша" },
  { value: "uz", label: "O‘zbekcha" },
  { value: "tr", label: "Türkçe" },
  { value: "zh", label: "中文" },
  { value: "hy", label: "Հայերեն" },
  { value: "fr", label: "Français" },
  { value: "pl", label: "Polski" },
  { value: "ar", label: "العربية" },
];

export function isAppLocale(value: unknown): value is AppLocale {
  return (
    typeof value === "string" &&
    (APP_LOCALES as readonly string[]).includes(value)
  );
}

export function normalizeAppLocale(value: unknown): AppLocale | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim().toLowerCase().replaceAll("_", "-");
  if (!normalized) return null;
  const base = normalized.split("-")[0];
  return isAppLocale(base) ? base : null;
}

export function detectAppLocale(
  storage: Pick<Storage, "getItem"> | null = browserStorage(),
  languages: readonly string[] = browserLanguages(),
): AppLocale {
  const stored = normalizeAppLocale(storage?.getItem(APP_LOCALE_STORAGE_KEY));
  if (stored) return stored;
  for (const language of languages) {
    const locale = normalizeAppLocale(language);
    if (locale) return locale;
  }
  return "en";
}

export function persistAppLocale(
  locale: AppLocale,
  storage: Pick<Storage, "setItem"> | null = browserStorage(),
): void {
  try {
    storage?.setItem(APP_LOCALE_STORAGE_KEY, locale);
  } catch {
    // Language selection must remain usable when storage is unavailable.
  }
}

export function appLocaleDirection(locale: AppLocale): "ltr" | "rtl" {
  return locale === "ar" ? "rtl" : "ltr";
}

export function legalDocumentsLocale(locale: AppLocale): "en" | "ru" {
  return locale === "ru" ? "ru" : "en";
}

function browserStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function browserLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  return navigator.languages.length
    ? navigator.languages
    : [navigator.language];
}
