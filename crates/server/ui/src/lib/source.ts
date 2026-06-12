// Build a provider deep-link for a source range from an opt-in template (A12).
// The hub stores/serves no source — this only points the browser at the repo's
// own provider. Pure + unit-tested.

export interface RangeRef {
  path: string;
  start: number;
  end: number;
}

export type Provider = "github" | "gitlab" | "bitbucket";

export interface BuiltTemplates {
  link: string;
  raw: string;
}

/**
 * Generate `link` + `raw` source templates from a provider + `owner/repo` slug +
 * branch, so an admin connects a repo without hand-writing template syntax. For
 * a private GitHub repo the raw template targets the authenticated contents API
 * (proxied + tokened server-side); the others target the public raw host. The
 * `{path}`/`{start}`/`{end}` placeholders are filled in later, per range.
 */
export function buildTemplates(
  provider: Provider,
  slug: string,
  branch: string,
  isPrivate: boolean,
): BuiltTemplates {
  const s = slug.trim().replace(/^\/+|\/+$/g, "");
  const b = branch.trim() || "main";
  switch (provider) {
    case "github":
      return {
        link: `https://github.com/${s}/blob/${b}/{path}#L{start}-L{end}`,
        raw: isPrivate
          ? `https://api.github.com/repos/${s}/contents/{path}?ref=${b}`
          : `https://raw.githubusercontent.com/${s}/${b}/{path}`,
      };
    case "gitlab":
      return {
        link: `https://gitlab.com/${s}/-/blob/${b}/{path}#L{start}-{end}`,
        raw: `https://gitlab.com/${s}/-/raw/${b}/{path}`,
      };
    case "bitbucket":
      return {
        link: `https://bitbucket.org/${s}/src/${b}/{path}#lines-{start}:{end}`,
        raw: `https://bitbucket.org/${s}/raw/${b}/{path}`,
      };
  }
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

/**
 * Build the raw-bytes URL for a file from a `{path}` template (https-only,
 * path-encoded per segment). The browser fetches this directly from the provider
 * to render the inline source gutter — the hub is never involved.
 */
export function rawUrl(template: string | null | undefined, path: string): string | null {
  if (!template) return null;
  const encodedPath = path
    .split("/")
    .map(encodeURIComponent)
    .join("/");
  const url = template.replaceAll("{path}", encodedPath);
  return url.startsWith("https://") ? url : null;
}

/**
 * Return source lines `[start, end]` (1-based, inclusive) from full file text,
 * each as `{ n, text }`. Out-of-range lines are skipped. Pure + bounded by the
 * caller's range, so a huge file only materialises the requested window.
 */
export function sliceLines(
  text: string,
  start: number,
  end: number,
): { n: number; text: string }[] {
  if (start < 1 || end < start) return [];
  const lines = text.split("\n");
  const out: { n: number; text: string }[] = [];
  for (let n = start; n <= end && n <= lines.length; n++) {
    out.push({ n, text: lines[n - 1] ?? "" });
  }
  return out;
}
