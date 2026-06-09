// Minimal org-scoped client router. URL is the source of truth; routes live
// under the /app base (see docs/proposals/TEAM_DASHBOARD_UI.md §3.2). Kept
// dependency-free and pure (parsing is unit-tested).

export type Route =
  | { name: "overview"; org: string }
  | { name: "repos"; org: string }
  | { name: "repo"; org: string; repo: string }
  | { name: "file"; org: string; repo: string; path: string }
  | { name: "sessions"; org: string }
  | { name: "session"; org: string; id: string }
  | { name: "unknown"; org: string | null; path: string };

const BASE = "/app";

/** Parse a pathname into a route. Org-scoped: /app/orgs/:org/<screen>. */
export function parseRoute(pathname: string): Route | null {
  let rest = pathname.startsWith(BASE) ? pathname.slice(BASE.length) : pathname;
  rest = rest.replace(/\/+$/, ""); // trailing slashes
  if (rest === "" || rest === "/") return null; // → caller redirects to default org

  const parts = rest.split("/").filter(Boolean); // ["orgs", ":org", ...]
  if (parts[0] !== "orgs" || parts.length < 2) {
    return { name: "unknown", org: null, path: pathname };
  }
  const org = parts[1]!;
  const screen = parts[2] ?? "overview";
  switch (screen) {
    case "overview":
      return { name: "overview", org };
    case "repos": {
      if (!parts[3]) return { name: "repos", org };
      const repo = decodeURIComponent(parts[3]);
      // /orgs/:org/repos/:repo/files/<path...>
      if (parts[4] === "files" && parts.length > 5) {
        const path = parts.slice(5).map(decodeURIComponent).join("/");
        return { name: "file", org, repo, path };
      }
      return { name: "repo", org, repo };
    }
    case "sessions":
      return parts[3]
        ? { name: "session", org, id: decodeURIComponent(parts[3]) }
        : { name: "sessions", org };
    default:
      return { name: "unknown", org, path: pathname };
  }
}

/** Build the canonical path for a route. */
export function routePath(route: Route): string {
  switch (route.name) {
    case "overview":
      return `${BASE}/orgs/${route.org}/overview`;
    case "repos":
      return `${BASE}/orgs/${route.org}/repos`;
    case "repo":
      return `${BASE}/orgs/${route.org}/repos/${encodeURIComponent(route.repo)}`;
    case "file":
      return filePath(route.org, route.repo, route.path);
    case "sessions":
      return `${BASE}/orgs/${route.org}/sessions`;
    case "session":
      return `${BASE}/orgs/${route.org}/sessions/${encodeURIComponent(route.id)}`;
    case "unknown":
      return route.path;
  }
}

/** Path to the sessions list for an org. */
export function sessionsPath(org: string): string {
  return `${BASE}/orgs/${org}/sessions`;
}

/** Path to a session's replay. */
export function sessionPath(org: string, id: string): string {
  return `${BASE}/orgs/${org}/sessions/${encodeURIComponent(id)}`;
}

/** Path to a file's provenance view (each path segment encoded). */
export function filePath(org: string, repo: string, path: string): string {
  const encoded = path.split("/").map(encodeURIComponent).join("/");
  return `${BASE}/orgs/${org}/repos/${encodeURIComponent(repo)}/files/${encoded}`;
}

/** Default landing path for an org. */
export function defaultPath(org: string): string {
  return `${BASE}/orgs/${org}/overview`;
}

/** Path to the repositories list for an org. */
export function reposPath(org: string): string {
  return `${BASE}/orgs/${org}/repos`;
}

/** Path to a single repo's detail. */
export function repoPath(org: string, repo: string): string {
  return `${BASE}/orgs/${org}/repos/${encodeURIComponent(repo)}`;
}

/** SPA navigation (history push) + notify listeners. */
export function navigate(path: string): void {
  if (path !== location.pathname) {
    history.pushState({}, "", path);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }
}
