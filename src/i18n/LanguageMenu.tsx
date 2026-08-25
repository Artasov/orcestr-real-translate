import { IconButton, Menu, Tooltip } from "@orcestr/ui";
import { LuCheck, LuLanguages } from "react-icons/lu";

import { useAppI18n } from "./I18nProvider";
import { APP_LOCALE_OPTIONS } from "./locales";

export function LanguageMenu({ className }: { className?: string }) {
  const { app, locale, setLocale } = useAppI18n();

  return (
    <Menu
      align="end"
      items={APP_LOCALE_OPTIONS.map((option) => ({
        key: option.value,
        label: option.label,
        info:
          option.value === locale ? (
            <LuCheck size={14} aria-hidden="true" />
          ) : null,
        onSelect: () => setLocale(option.value),
      }))}
      trigger={
        <Tooltip content={app.common.chooseLanguage}>
          <IconButton
            type="button"
            icon={<LuLanguages size={17} />}
            aria-label={app.common.chooseLanguage}
            v="ghost"
            tone="neutral"
            size={2}
            className={className}
          />
        </Tooltip>
      }
      testId="language-menu"
    />
  );
}
