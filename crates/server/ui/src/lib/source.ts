// Build a provider deep-link for a source range from an opt-in template (A12).
// The hub stores/serves no source — this only points the browser at the repo's
// own provider. Pure + unit-tested.

export interface RangeRef {
  path: string;
  start: number;
  end: number;
}

/**
 * Substitute `{path}` / `{start}` / `{end}` in `template`. The path is
 * URL-encoded per segment (slashes preserved) so filenames containing `#`, `?`
 * or `%` don't corrupt the link. Returns null for an empty template or one that
 * doesn't resolve to an `https://` URL (defence-in-depth even though the API
 * validates). No commit/blob ref is substituted — the attribution model has no
 * commit, so templates pin the ref themselves (e.g. `.../blob/main/{path}`).
 */
export function sourceLink(template: string | null | undefined, r: RangeRef): string | null {
  if (!template) return null;
  const encodedPath = r.path
    .split("/")
    .map(encodeURIComponent)
    .join("/");
  const url = template
    .replaceAll("{path}", encodedPath)
    .replaceAll("{start}", String(r.start))
    .replaceAll("{end}", String(r.end));
  return url.startsWith("https://") ? url : null;
}
