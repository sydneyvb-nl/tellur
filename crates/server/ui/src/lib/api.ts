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

async function handle<T>(res: Response): Promise<T> {
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

async function request<T>(path: string): Promise<T> {
  return handle<T>(
    await fetch(path, {
      credentials: "include",
      headers: { Accept: "application/json" },
    }),
  );
}

/** POST with no body (used to enqueue durable export jobs → 202 + job_id). */
async function post<T>(path: string): Promise<T> {
  return handle<T>(
    await fetch(path, {
      method: "POST",
      credentials: "include",
      headers: { Accept: "application/json" },
    }),
  );
}

/** PUT with a JSON body (used for admin settings like the source connection). */
async function put<T>(path: string, body: unknown): Promise<T> {
  return handle<T>(
    await fetch(path, {
      method: "PUT",
      credentials: "include",
      headers: { Accept: "application/json", "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  );
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

export interface AttrRange {
  start_line: number;
  end_line: number;
  origin: string;
  agent_id: string;
  model_id: string | null;
  confidence: number;
  reviewer: string | null;
  reviewed_at: string | null;
}

export interface AttrFile {
  file_path: string;
  git_blob_sha: string;
  ranges: AttrRange[];
}

export interface SessionSummary {
  session_id: string;
  event_count: number;
  first_ts: string;
  last_ts: string;
  actors: string[];
  repos: string[];
}

export interface AuditRecord {
  seq: number;
  ts: string;
  org_id: string | null;
  actor_member_id: string | null;
  action: string;
  detail: string;
  entry_hash: string;
}

export interface AuditPage {
  org_id: string;
  // Whether the tamper-evident chain still verifies (only on the first page).
  chain_intact: boolean | null;
  // Cursor for the next (older) page, or null at the end.
  next_before: number | null;
  records: AuditRecord[];
}

export interface Job {
  id: string;
  org_id: string;
  kind: string;
  status: string;
  result: string | null;
  error: string | null;
  created_at: string;
  updated_at: string;
}

export type ExportKind = "events" | "audit" | "evidence";

export interface OverviewRepo {
  id: string;
  name: string;
  event_count: number;
  ai_lines: number;
  reviewed_ai_lines: number;
  review_gap_lines: number;
  ai_share: number | null;
  review_coverage: number | null;
}

export interface Overview {
  org_id: string;
  generated_at: string;
  totals: {
    events: number;
    sessions: number;
    repos: number;
    ai_lines: number;
    reviewed_ai_lines: number;
    total_attributed_lines: number;
  };
  ai_share: number | null;
  review_coverage: number | null;
  activity: ActivityBucket[];
  repos: OverviewRepo[];
  recent_events: StoredEvent[];
}

export interface ComplianceSnapshot {
  repo_id: string;
  repo_name: string;
  policy_name: string;
  policy_version: number;
  evaluated_at: string;
  ai_ranges: number;
  violations: number;
  high: number;
  medium: number;
  low: number;
}

export interface CompliancePage {
  org_id: string;
  evaluated: boolean;
  snapshots: ComplianceSnapshot[];
}

export interface MemberInfo {
  id: string;
  display_name: string;
  role: string;
  email: string | null;
  sso_bound: boolean;
  active: boolean;
}

export interface GroupInfo {
  id: string;
  display_name: string;
  external_id: string | null;
  members: string[];
  maps_to_role: string | null;
}

export interface SsoStatus {
  oidc_enabled: boolean;
  oidc_issuer: string | null;
  scim_configured: boolean;
  scim_token_created_at: string | null;
  members_total: number;
  members_active: number;
  members_sso_bound: number;
  scim_groups: number;
}

export interface SourceConfig {
  repo_id: string;
  source_template: string | null;
  source_raw_template: string | null;
  token_configured: boolean;
}

export interface SourceUpdate {
  template?: string | null;
  raw_template?: string | null;
  /** Non-empty sets the proxy token; omit to preserve the existing one. */
  token?: string;
  /** Remove any stored proxy token. */
  clear_token?: boolean;
}

const org = (o: string) => encodeURIComponent(o);

export const api = {
  me: () => request<Me>("/v1/me"),
  dashboard: (o: string, limit = 25) =>
    request<Dashboard>(`/v1/orgs/${org(o)}/dashboard?limit=${limit}`),
  overview: (o: string) => request<Overview>(`/v1/orgs/${org(o)}/overview`),
  activity: (o: string, rangeDays = 30, groupBy: "type" | "actor" = "type") =>
    request<Activity>(
      `/v1/orgs/${org(o)}/activity?range=${rangeDays}d&group_by=${groupBy}`,
    ),
  repo: (o: string, repo: string) =>
    request<RepoDetail>(`/v1/orgs/${org(o)}/repos/${encodeURIComponent(repo)}`),
  attributions: (o: string, repo: string) =>
    request<{
      repo_id: string;
      files: AttrFile[];
      source_template: string | null;
      source_raw_template: string | null;
      source_proxy: boolean;
    }>(`/v1/orgs/${org(o)}/repos/${encodeURIComponent(repo)}/attributions`),
  // Source connection (A12, admin): read the current config (token never
  // returned — only `token_configured`), set/update it, or fetch a file's raw
  // bytes through the hub's proxy for private repos.
  getSource: (o: string, repo: string) =>
    request<SourceConfig>(
      `/v1/orgs/${org(o)}/repos/${encodeURIComponent(repo)}/source`,
    ),
  setSource: (o: string, repo: string, body: SourceUpdate) =>
    put<SourceConfig>(
      `/v1/orgs/${org(o)}/repos/${encodeURIComponent(repo)}/source`,
      body,
    ),
  blob: (o: string, repo: string, path: string) =>
    request<{ path: string; content: string }>(
      `/v1/orgs/${org(o)}/repos/${encodeURIComponent(repo)}/blob?path=${encodeURIComponent(path)}`,
    ),
  sessions: (o: string, repo?: string) =>
    request<{ sessions: SessionSummary[] }>(
      `/v1/orgs/${org(o)}/sessions${repo ? `?repo=${encodeURIComponent(repo)}` : ""}`,
    ),
  session: (o: string, id: string) =>
    request<{ session_id: string; events: StoredEvent[]; truncated?: boolean }>(
      `/v1/orgs/${org(o)}/sessions/${encodeURIComponent(id)}`,
    ),
  audit: (
    o: string,
    opts: { actor?: string; action?: string; rangeDays?: number; before?: number } = {},
  ) => {
    const q = new URLSearchParams();
    if (opts.actor) q.set("actor", opts.actor);
    if (opts.action) q.set("action", opts.action);
    if (opts.rangeDays) q.set("range", `${opts.rangeDays}d`);
    if (opts.before != null) q.set("before", String(opts.before));
    const qs = q.toString();
    return request<AuditPage>(`/v1/orgs/${org(o)}/audit${qs ? `?${qs}` : ""}`);
  },
  jobs: (o: string) => request<{ jobs: Job[] }>(`/v1/orgs/${org(o)}/jobs`),
  job: (o: string, id: string) =>
    request<Job>(`/v1/orgs/${org(o)}/jobs/${encodeURIComponent(id)}`),
  startExport: (o: string, kind: ExportKind) =>
    post<{ job_id: string; status: string; poll: string }>(
      `/v1/orgs/${org(o)}/export/${kind}`,
    ),
  compliance: (o: string) =>
    request<CompliancePage>(`/v1/orgs/${org(o)}/policies/compliance`),
  runCompliance: (o: string) =>
    post<{ job_id: string; status: string; poll: string }>(
      `/v1/orgs/${org(o)}/policies/compliance`,
    ),
  members: (o: string) =>
    request<{ members: MemberInfo[] }>(`/v1/orgs/${org(o)}/members`),
  groups: (o: string) =>
    request<{ groups: GroupInfo[] }>(`/v1/orgs/${org(o)}/groups`),
  ssoStatus: (o: string) => request<SsoStatus>(`/v1/orgs/${org(o)}/sso-status`),
};
