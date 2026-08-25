import { authMessages as sharedAuthMessages } from "@orcestr/auth-forms";
import { describe, expect, it } from "vitest";

import { APP_MESSAGE_ROWS } from "./app-message-rows";
import { APP_LOCALES } from "./locales";
import { localeMessages } from "./messages";

describe("localized application copy", () => {
  it("contains a non-empty translation for every row and locale", () => {
    for (const [key, ...translations] of APP_MESSAGE_ROWS) {
      expect(translations, key).toHaveLength(APP_LOCALES.length);
      for (const translation of translations) {
        expect(translation.trim(), key).not.toBe("");
      }
    }
  });

  it("preserves interpolation placeholders in every translation", () => {
    for (const [key, source, ...translations] of APP_MESSAGE_ROWS) {
      const expected = placeholders(source);
      for (const translation of translations) {
        expect(placeholders(translation), key).toEqual(expected);
      }
    }
  });

  it("matches the complete application, auth-form and UI-copy shapes", () => {
    const appShape = leafPaths(localeMessages.en.app);
    const authShape = leafPaths(sharedAuthMessages.en);
    const requiredUiKeys = [
      "clear",
      "close",
      "retry",
      "loading",
      "noOptions",
      "selectValue",
      "clearSelectedValues",
      "details",
      "scrollTabsLeft",
      "scrollTabsRight",
    ];

    for (const locale of APP_LOCALES) {
      const messages = localeMessages[locale];
      expect(leafPaths(messages.app), locale).toEqual(appShape);
      expect(leafPaths(messages.auth), locale).toEqual(authShape);
      for (const key of requiredUiKeys) {
        const value =
          messages.ui.common?.[key as keyof typeof messages.ui.common];
        expect(typeof value, `${locale}.ui.common.${key}`).toBe("string");
        expect(String(value).trim(), `${locale}.ui.common.${key}`).not.toBe("");
      }
    }
  });

  it("does not fall back to English for the primary visible copy", () => {
    for (const locale of APP_LOCALES.filter((value) => value !== "en")) {
      expect(localeMessages[locale].app.auth.title).not.toBe(
        localeMessages.en.app.auth.title,
      );
      expect(localeMessages[locale].app.workspace.start).not.toBe(
        localeMessages.en.app.workspace.start,
      );
    }
  });
});

function placeholders(value: string): string[] {
  return [...value.matchAll(/\{[a-zA-Z][a-zA-Z0-9]*\}/gu)]
    .map(([placeholder]) => placeholder)
    .sort();
}

function leafPaths(value: unknown, prefix = ""): string[] {
  if (!value || typeof value !== "object") return [prefix];
  return Object.entries(value)
    .flatMap(([key, item]) =>
      leafPaths(item, prefix ? `${prefix}.${key}` : key),
    )
    .sort();
}
