import { test, expect, type Page } from "@playwright/test";

// The SPA fetches everything from the authenticated /v1 API with a session
// cookie, which a headless browser can't obtain. So we intercept /v1 and return
// fixtures: this exercises the real built bundle (routing, rendering, role
// gating, i18n/theme, command palette) without a live hub.

const ORG = "test";

function fixtures(role: "admin" | "viewer") {
  return {
    me: { org_id: ORG, member_id: "m1", role },
    overview: {
      org_id: ORG,
      generated_at: new Date().toISOString(),
      totals: {
        events: 10,
        sessions: 3,
        repos: 1,
        ai_lines: 20,
        reviewed_ai_lines: 8,
        total_attributed_lines: 20,
      },
      ai_share: 1,
      review_coverage: 0.4,
      activity: [{ day: "2026-06-01", key: "file.write", count: 3 }],
      repos: [
        {
          id: "r1",
          name: "app",
          event_count: 5,
          ai_lines: 20,
          reviewed_ai_lines: 8,
          review_gap_lines: 12,
          ai_share: 1,
          review_coverage: 0.4,
        },
      ],
      recent_events: [
        {
          id: "e1",
          seq: 1,
          repo_id: "r1",
          session_id: "s1",
          timestamp: new Date().toISOString(),
          type: "file.write",
          actor: "claude",
          payload: {},
        },
      ],
    },
    members: {
      org_id: ORG,
      members: [
        {
          id: "m1",
          display_name: "Alice",
          role: "admin",
          email: "alice@corp.test",
          sso_bound: true,
          active: true,
        },
      ],
    },
    groups: {
      org_id: ORG,
      groups: [
        {
          id: "g1",
          display_name: "tellur-admin",
          external_id: null,
          members: ["m1"],
          maps_to_role: "admin",
        },
      ],
    },
    sso: {
      org_id: ORG,
      oidc_enabled: false,
      oidc_issuer: null,
      scim_configured: false,
      scim_token_created_at: null,
      members_total: 1,
      members_active: 1,
      members_sso_bound: 1,
      scim_groups: 1,
    },
  };
}

async function mockApi(page: Page, role: "admin" | "viewer" = "admin") {
  const fx = fixtures(role);
  const json = (body: unknown) => ({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
  await page.route("**/v1/**", (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith("/v1/me")) return route.fulfill(json(fx.me));
    if (path.endsWith("/overview")) return route.fulfill(json(fx.overview));
    if (path.endsWith("/members")) return route.fulfill(json(fx.members));
    if (path.endsWith("/groups")) return route.fulfill(json(fx.groups));
    if (path.endsWith("/sso-status")) return route.fulfill(json(fx.sso));
    // Fail loudly on anything unmocked (a typo or new endpoint) so a screen's
    // data fetch landing on the wrong URL surfaces as an error, not a false pass.
    return route.fulfill({
      status: 404,
      contentType: "application/json",
      body: JSON.stringify({ title: `unmocked: ${path}` }),
    });
  });
}

test("overview renders and admin nav is present", async ({ page }) => {
  await mockApi(page, "admin");
  await page.goto(`/app/orgs/${ORG}/overview`);

  await expect(page.getByRole("heading", { name: "Overview", level: 1 })).toBeVisible();
  // KPI value from the mocked overview payload.
  await expect(page.locator(".kpi").filter({ hasText: "Events" })).toContainText("10");
  // Admin-only rail items are visible for an admin.
  await expect(page.getByRole("link", { name: "Policies" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Audit log" })).toBeVisible();
});

test("non-admin does not see admin nav items", async ({ page }) => {
  await mockApi(page, "viewer");
  await page.goto(`/app/orgs/${ORG}/overview`);

  await expect(page.getByRole("link", { name: "Overview" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Policies" })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Audit log" })).toHaveCount(0);
});

test("language switch translates the UI live", async ({ page }) => {
  await mockApi(page, "admin");
  await page.goto(`/app/orgs/${ORG}/overview`);

  await expect(page.getByRole("link", { name: "Overview" })).toBeVisible();
  await page.getByRole("button", { name: "Language" }).click();
  // Dutch catalog: nav.overview → "Overzicht".
  await expect(page.getByRole("link", { name: "Overzicht" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Overview" })).toHaveCount(0);
});

test("theme toggle sets data-theme", async ({ page }) => {
  await mockApi(page, "admin");
  await page.goto(`/app/orgs/${ORG}/overview`);

  const theme = page.getByRole("button", { name: "Theme" });
  await expect(theme).toHaveText(/Auto/);
  await theme.click(); // Auto → Light
  await expect(theme).toHaveText(/Light/);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("command palette navigates to a screen", async ({ page }) => {
  await mockApi(page, "admin");
  await page.goto(`/app/orgs/${ORG}/overview`);
  await expect(page.getByRole("heading", { name: "Overview", level: 1 })).toBeVisible();

  await page.keyboard.press("Control+k");
  await page.getByRole("textbox", { name: "Search commands" }).fill("people");
  await page.keyboard.press("Enter");

  await expect(page).toHaveURL(new RegExp(`/app/orgs/${ORG}/people`));
  await expect(page.getByRole("heading", { name: "People & Access", level: 1 })).toBeVisible();
  // Assert data-derived content too, so a wrong/failed fetch (error state) can't
  // pass on the always-rendered heading alone.
  await expect(page.getByText("alice@corp.test")).toBeVisible();
});
