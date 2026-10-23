// SPDX-License-Identifier: GPL-3.0-or-later
import { createI18n } from "vue-i18n";
import en from "./locales/en.json";
import he from "./locales/he.json";

export type Locale = "he" | "en";
export const SUPPORTED_LOCALES: Locale[] = ["he", "en"];
export const RTL_LOCALES: Locale[] = ["he"];
export const STORAGE_KEY = "bfsw.locale";

/** Saved choice, else Hebrew (project default). */
export function initialLocale(): Locale {
  try {
    const saved = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (saved && (SUPPORTED_LOCALES as string[]).includes(saved)) return saved as Locale;
  } catch {
    /* storage unavailable */
  }
  return "he";
}

export const i18n = createI18n({
  legacy: false,
  locale: initialLocale(),
  fallbackLocale: "en",
  messages: { en, he },
});

export function isRtl(locale: string): boolean {
  return (RTL_LOCALES as string[]).includes(locale);
}

/** Set <html lang/dir> to match the locale. Layout uses logical CSS properties only. */
export function applyDirection(locale: string): void {
  if (typeof document === "undefined") return;
  document.documentElement.lang = locale;
  document.documentElement.dir = isRtl(locale) ? "rtl" : "ltr";
}

export function setLocale(locale: Locale): void {
  i18n.global.locale.value = locale;
  applyDirection(locale);
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, locale);
  } catch {
    /* storage unavailable */
  }
}
