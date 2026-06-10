// Reactive locale state + `t()` for components. Reading `t(...)` in markup tracks
// the locale rune, so the whole UI re-renders when the language switches. The
// pure catalog/translate live in `./i18n` (framework-free, unit-tested).

import {
  loadLocale,
  saveLocale,
  translate,
  type Locale,
  type Vars,
} from "./i18n";

let locale = $state<Locale>(loadLocale());

/** Current locale (reactive). */
export function getLocale(): Locale {
  return locale;
}

/** Switch + persist the active locale. */
export function setLocale(l: Locale): void {
  locale = l;
  saveLocale(l);
}

/** Apply the saved locale to <html lang> at boot (no state change). */
export function initLocale(): void {
  saveLocale(locale);
}

/** Translate `key` in the active locale (reactive on locale changes). */
export function t(key: string, vars?: Vars): string {
  return translate(locale, key, vars);
}
