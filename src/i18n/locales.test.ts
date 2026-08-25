import { describe, expect, it } from "vitest";

import {
  APP_LOCALES,
  APP_LOCALE_STORAGE_KEY,
  appLocaleDirection,
  detectAppLocale,
  legalDocumentsLocale,
  normalizeAppLocale,
  persistAppLocale,
} from "./locales";

describe("application locale routing", () => {
  it("uses the same eleven locales as the Development landing", () => {
    expect(APP_LOCALES).toEqual([
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
    ]);
  });

  it("normalizes regional browser locales and rejects unsupported ones", () => {
    expect(normalizeAppLocale("fr-CA")).toBe("fr");
    expect(normalizeAppLocale("zh_CN")).toBe("zh");
    expect(normalizeAppLocale("AR-sa")).toBe("ar");
    expect(normalizeAppLocale("de-DE")).toBeNull();
  });

  it("prefers a stored selection, then browser languages, then English", () => {
    expect(detectAppLocale({ getItem: () => "ka" }, ["fr-FR"])).toBe("ka");
    expect(detectAppLocale({ getItem: () => null }, ["de-DE", "pl-PL"])).toBe(
      "pl",
    );
    expect(detectAppLocale({ getItem: () => "invalid" }, ["de-DE"])).toBe("en");
  });

  it("persists the explicit choice and only Arabic uses RTL", () => {
    const values = new Map<string, string>();
    persistAppLocale("hy", {
      setItem: (key, value) => {
        values.set(key, value);
      },
    });
    expect(values.get(APP_LOCALE_STORAGE_KEY)).toBe("hy");
    expect(
      APP_LOCALES.filter((locale) => appLocaleDirection(locale) === "rtl"),
    ).toEqual(["ar"]);
  });

  it("uses Russian legal documents only for Russian UI", () => {
    expect(legalDocumentsLocale("ru")).toBe("ru");
    expect(legalDocumentsLocale("ka")).toBe("en");
  });
});
