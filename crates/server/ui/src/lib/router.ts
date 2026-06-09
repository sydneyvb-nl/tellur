// Minimal org-scoped client router. URL is the source of truth; routes live
// under the /app base (see docs/proposals/TEAM_DASHBOARD_UI.md §3.2). Kept
// dependency-free and pure (parsing is unit-tested).

export type Route =
  | { name: "overview"; org: string }
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
    default:
      return { name: "unknown", org, path: pathname };
  }
}

/** Build the canonical path for a route. */
export function routePath(route: Route): string {
  switch (route.name) {
    case "overview":
      return `${BASE}/orgs/${route.org}/overview`;
    case "unknown":
      return route.path;
  }
}

/** Default landing path for an org. */
export function defaultPath(org: string): string {
  return `${BASE}/orgs/${org}/overview`;
}

/** SPA navigation (history push) + notify listeners. */
export function navigate(path: string): void {
  if (path !== location.pathname) {
    history.pushState({}, "", path);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }
}
