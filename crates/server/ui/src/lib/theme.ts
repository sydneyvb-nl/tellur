// Theme preference handling. The CSS exposes a dark baseline (`:root`) and a
// light override (`:root[data-theme="light"]`); we resolve the user's preference
// to a concrete theme and reflect it on <html data-theme>. Pure resolution is
// unit-tested; the apply/load/save helpers touch the DOM and localStorage.

export type ThemePref = "system" | "light" | "dark";
export type Theme = "light" | "dark";

const KEY = "tellur.theme";
const PREFS: ThemePref[] = ["system", "light", "dark"];

/** Resolve a preference to a concrete theme given the OS dark-mode signal. */
export function resolveTheme(pref: ThemePref, prefersDark: boolean): Theme {
  if (pref === "system") return prefersDark ? "dark" : "light";
  return pref;
}

/** Validate/normalise an arbitrary stored value into a preference. */
export function normalizePref(raw: string | null): ThemePref {
  return PREFS.includes(raw as ThemePref) ? (raw as ThemePref) : "system";
}

/** The next preference in the cycle system → light → dark → system. */
export function nextPref(pref: ThemePref): ThemePref {
  return PREFS[(PREFS.indexOf(pref) + 1) % PREFS.length]!;
}

/** Read the saved preference (defaults to "system"). */
export function loadPref(): ThemePref {
  try {
    return normalizePref(localStorage.getItem(KEY));
  } catch {
    return "system";
  }
}

/** Persist + apply a preference to the document. Returns the resolved theme. */
export function applyPref(pref: ThemePref): Theme {
  const prefersDark =
    typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
  const theme = resolveTheme(pref, prefersDark);
  document.documentElement.setAttribute("data-theme", theme);
  try {
    localStorage.setItem(KEY, pref);
  } catch {
    /* storage unavailable (private mode) — theme still applies for this session */
  }
  return theme;
}
