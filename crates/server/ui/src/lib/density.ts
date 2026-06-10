// Display-density preference. Drives spacing tokens (--row-pad-*, --card-pad)
// via <html data-density>. Pure normalize/toggle are unit-tested; load/apply
// touch the DOM and localStorage.

export type Density = "comfortable" | "compact";

const KEY = "tellur.density";

/** Validate an arbitrary stored value (defaults to comfortable). */
export function normalizeDensity(raw: string | null): Density {
  return raw === "compact" ? "compact" : "comfortable";
}

/** Toggle between the two densities. */
export function toggleDensity(d: Density): Density {
  return d === "compact" ? "comfortable" : "compact";
}

/** Read the saved density (defaults to comfortable). */
export function loadDensity(): Density {
  try {
    return normalizeDensity(localStorage.getItem(KEY));
  } catch {
    return "comfortable";
  }
}

/** Persist + apply a density to the document. */
export function applyDensity(d: Density): void {
  // Comfortable is the CSS default, so only the compact attribute is set.
  if (d === "compact") document.documentElement.setAttribute("data-density", "compact");
  else document.documentElement.removeAttribute("data-density");
  try {
    localStorage.setItem(KEY, d);
  } catch {
    /* storage unavailable — density still applies for this session */
  }
}
