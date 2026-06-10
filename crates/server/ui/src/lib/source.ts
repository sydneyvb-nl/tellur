// Build a provider deep-link for a source range from an opt-in template (A12).
// The hub stores/serves no source — this only points the browser at the repo's
// own provider. Pure + unit-tested.

export interface RangeRef {
  path: string;
  start: number;
  end: number;
  sha: string;
}

/**
 * Substitute `{path}` / `{start}` / `{end}` / `{sha}` in `template`. Returns
 * null for an empty template or one that doesn't resolve to an `https://` URL
 * (defence-in-depth against a non-https href even though the API validates).
 */
export function sourceLink(template: string | null | undefined, r: RangeRef): string | null {
  if (!template) return null;
  const url = template
    .replaceAll("{path}", r.path)
    .replaceAll("{start}", String(r.start))
    .replaceAll("{end}", String(r.end))
    .replaceAll("{sha}", r.sha);
  return url.startsWith("https://") ? url : null;
}
