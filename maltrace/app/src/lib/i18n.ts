/// Lightweight i18n shim.
///
/// The English strings live inline as defaults — most of the codebase
/// uses literal strings today. Calling `t("save", "Save")` lets a
/// future translation drop a `pt-BR.json` etc. into the catalogues
/// map without touching call sites. This keeps the scaffolding cheap
/// (no bundle bloat for users who don't change locale) while opening
/// the door to localised builds.
///
/// `setLocale(code)` persists to localStorage; `getLocale()` reads
/// it. Defaults to `"en"`. Callers re-render via the standard React
/// patterns — since translations are static maps, no Provider is
/// needed; `t()` is a pure function.

const LOCALE_KEY = "maltrace_locale";

/// Add a catalogue here when shipping a translation pack. The shape
/// is `{ key: string }`; keys not present fall back to the English
/// default the caller supplied.
const CATALOGUES: Record<string, Record<string, string>> = {
  en: {},
  // Example (commented out — real packs would be a JSON import):
  // "pt-BR": { save: "Salvar", cancel: "Cancelar" },
};

let cachedLocale: string | null = null;

export function getLocale(): string {
  if (cachedLocale !== null) return cachedLocale;
  try {
    cachedLocale = localStorage.getItem(LOCALE_KEY) ?? "en";
  } catch {
    cachedLocale = "en";
  }
  return cachedLocale;
}

export function setLocale(code: string) {
  cachedLocale = code;
  try {
    localStorage.setItem(LOCALE_KEY, code);
  } catch {
    /* private mode / no DOM */
  }
}

export function availableLocales(): string[] {
  return Object.keys(CATALOGUES);
}

/// Look up a translated string. `fallback` is the English default,
/// returned verbatim if no translation is registered. Always returns
/// a string — never throws on missing keys.
export function t(key: string, fallback: string): string {
  const cat = CATALOGUES[getLocale()];
  if (cat && key in cat) return cat[key];
  return fallback;
}
