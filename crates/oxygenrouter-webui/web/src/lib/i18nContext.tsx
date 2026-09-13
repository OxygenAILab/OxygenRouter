import React from "react";
import { Locale, STRINGS, Strings } from "./i18n";

const STORAGE_KEY = "oxygenrouter:locale";

function detectLocale(): Locale {
  if (typeof window === "undefined") return "en";
  const stored = window.localStorage.getItem(STORAGE_KEY);
  if (stored === "en" || stored === "zh") return stored;
  const lang = window.navigator?.language?.toLowerCase() ?? "";
  return lang.startsWith("zh") ? "zh" : "en";
}

interface I18nContextValue {
  locale: Locale;
  setLocale: (value: Locale) => void;
  t: Strings;
}

const I18nContext = React.createContext<I18nContextValue>({
  locale: "en",
  setLocale: () => undefined,
  t: STRINGS.en,
});

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [locale, setLocaleState] = React.useState<Locale>(detectLocale);
  const setLocale = React.useCallback((value: Locale) => {
    setLocaleState(value);
    if (typeof window !== "undefined") window.localStorage.setItem(STORAGE_KEY, value);
  }, []);
  const t = (locale === "zh" ? STRINGS.zh : STRINGS.en) as unknown as Strings;
  return <I18nContext.Provider value={{ locale, setLocale, t }}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  return React.useContext(I18nContext);
}
