// Thin JSON API client for the hub. Same-origin; the browser sends the SSO
// session cookie (credentials: include). A 401 redirects to the SSO login,
// preserving the current path so the user lands back where they were.

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string): Promise<T> {
  const res = await fetch(path, {
    credentials: "include",
    headers: { Accept: "application/json" },
  });
  if (res.status === 401) {
    const target = location.pathname + location.search;
    location.assign(`/auth/login?return=${encodeURIComponent(target)}`);
    throw new ApiError(401, "redirecting to sign in");
  }
  if (!res.ok) {
    // The hub returns RFC 9457 problem+json; surface its title when present.
    let detail = `request failed (${res.status})`;
    try {
      const body = await res.json();
      detail = body.title || body.detail || detail;
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(res.status, detail);
  }
  return (await res.json()) as T;
}

// ── Typed views of the hub payloads we consume in D0 ────────────────────────

export interface Me {
  org_id: string;
  member_id: string;
  role: string;
}

export interface RepoSummary {
  id: string;
  name: string;
  event_count: number;
}

export interface StoredEvent {
  seq: number;
  id: string;
  repo_id: string;
  session_id: string;
  timestamp: string;
  type: string;
  actor: string;
  payload: unknown;
}

export interface Dashboard {
  org_id: string;
  generated_at: string;
  report: {
    total_events: number;
    distinct_sessions: number;
    by_type: Record<string, number>;
    by_actor: Record<string, number>;
    repos: RepoSummary[];
  };
  recent_events: StoredEvent[];
}

export interface ActivityBucket {
  day: string;
  key: string;
  count: number;
}

export interface Activity {
  org_id: string;
  range_days: number;
  group_by: string;
  buckets: ActivityBucket[];
}

export interface RepoDetail {
  id: string;
  name: string;
  event_count: number;
  contributors: string[];
  last_activity: string | null;
  attributed_files: number;
  lines: { total_attributed: number; ai: number; reviewed_ai: number };
  ai_share: number | null;
  review_coverage: number | null;
}

const org = (o: string) => encodeURIComponent(o);

export const api = {
  me: () => request<Me>("/v1/me"),
  dashboard: (o: string, limit = 25) =>
    request<Dashboard>(`/v1/orgs/${org(o)}/dashboard?limit=${limit}`),
  activity: (o: string, rangeDays = 30, groupBy: "type" | "actor" = "type") =>
    request<Activity>(
      `/v1/orgs/${org(o)}/activity?range=${rangeDays}d&group_by=${groupBy}`,
    ),
  repo: (o: string, repo: string) =>
    request<RepoDetail>(`/v1/orgs/${org(o)}/repos/${encodeURIComponent(repo)}`),
};
