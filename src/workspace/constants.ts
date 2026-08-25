import type { SelectItem } from "@orcestr/ui";

import type { AppLocale } from "../i18n/locales";

const TARGET_LANGUAGES = [
  "en",
  "ru",
  "es",
  "de",
  "fr",
  "it",
  "pt",
  "pl",
  "uk",
  "tr",
  "ar",
  "hi",
  "zh",
  "ja",
  "ko",
  "nl",
  "sv",
  "cs",
  "el",
  "he",
] as const;

const FALLBACK_LANGUAGE_NAMES: Record<
  (typeof TARGET_LANGUAGES)[number],
  string
> = {
  en: "English",
  ru: "Russian",
  es: "Spanish",
  de: "German",
  fr: "French",
  it: "Italian",
  pt: "Portuguese",
  pl: "Polish",
  uk: "Ukrainian",
  tr: "Turkish",
  ar: "Arabic",
  hi: "Hindi",
  zh: "Chinese",
  ja: "Japanese",
  ko: "Korean",
  nl: "Dutch",
  sv: "Swedish",
  cs: "Czech",
  el: "Greek",
  he: "Hebrew",
};

const languageItemCache = new Map<
  AppLocale,
  ReadonlyArray<SelectItem<string>>
>();

export function languageItems(
  locale: AppLocale,
): ReadonlyArray<SelectItem<string>> {
  const cached = languageItemCache.get(locale);
  if (cached) return cached;

  let displayNames: Intl.DisplayNames | null = null;
  try {
    displayNames = new Intl.DisplayNames([locale, "en"], { type: "language" });
  } catch {
    // The WebView may not ship every Intl locale; English names remain usable.
  }

  const items = TARGET_LANGUAGES.map((value) => ({
    value,
    label: displayNames?.of(value) ?? FALLBACK_LANGUAGE_NAMES[value],
  }));
  languageItemCache.set(locale, items);
  return items;
}
