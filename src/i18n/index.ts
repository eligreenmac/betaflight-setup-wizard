import { createI18n } from "vue-i18n";
import en from "./locales/en.json";
import he from "./locales/he.json";

export type Locale = "he" | "en";
export const RTL_LOCALES: Locale[] = ["he"];

export const i18n = createI18n({
  legacy: false,
  locale: "he" as Locale,
  fallbackLocale: "en",
  messages: { en, he },
});

export function isRtl(locale: string): boolean {
  return (RTL_LOCALES as string[]).includes(locale);
}

/** Set <html lang/dir> to match the locale. */
export function applyDirection(locale: string): void {
  if (typeof document === "undefined") return;
  document.documentElement.lang = locale;
  document.documentElement.dir = isRtl(locale) ? "rtl" : "ltr";
}

export function setLocale(locale: Locale): void {
  i18n.global.locale.value = locale;
  applyDirection(locale);
}
