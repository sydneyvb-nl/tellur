// Internationalisation: a flat message catalog per locale + a pure `translate`.
// Keys are dotted by screen. `translate` interpolates `{var}` placeholders and
// falls back to English for any key missing in another locale, returning the key
// itself if it's unknown everywhere (a visible signal during development).
//
// The reactive `t()` / locale state lives in `i18n.svelte.ts`; this module stays
// framework-free so it can be unit-tested directly.

export type Locale = "en" | "nl";
export const LOCALES: Locale[] = ["en", "nl"];

const KEY = "tellur.locale";

export type Vars = Record<string, string | number>;

const en: Record<string, string> = {
  // Common
  "common.retry": "Retry",
  "common.loading": "Loading…",
  "common.by": "by {actor}",
  "common.ai": "AI",
  "common.human": "human",
  "common.mixed": "mixed",

  // App shell / nav
  "nav.overview": "Overview",
  "nav.repos": "Repositories",
  "nav.sessions": "Sessions",
  "nav.policies": "Policies",
  "nav.people": "People & Access",
  "nav.exports": "Exports",
  "nav.audit": "Audit log",
  "nav.soon": "soon",
  "shell.skip": "Skip to content",
  "shell.search": "Search",
  "shell.signOut": "Sign out",
  "shell.org": "Organization",
  "shell.role": "Your role",
  "shell.densityTitle": "Display density",
  "shell.themeTitle": "Theme",
  "shell.langTitle": "Language",
  "density.comfortable": "Cozy",
  "density.compact": "Compact",
  "theme.system": "Auto",
  "theme.light": "Light",
  "theme.dark": "Dark",

  // Command palette
  "palette.label": "Command palette",
  "palette.placeholder": "Jump to…",
  "palette.search": "Search commands",
  "palette.none": "No matches",
  "palette.navigate": "navigate",
  "palette.open": "open",
  "palette.close": "close",
  "hint.compliance": "compliance",
  "hint.admin": "admin",

  // App-level
  "app.notFound": "Not found",
  "app.backToOverview": "Back to overview",
  "app.failed": "failed to load",

  // Trend
  "trend.activity": "Activity ({days} days)",
  "trend.noActivity": "No activity in range.",

  // Overview
  "overview.title": "Overview",
  "overview.emptyTitle": "No activity yet for this org.",
  "overview.emptyHintPre": "Connect a repo and push provenance, e.g.",
  "overview.emptyHintPost": "or the hub ingest API.",
  "overview.kpiEvents": "Events",
  "overview.kpiSessions": "Sessions",
  "overview.kpiRepos": "Repositories",
  "overview.kpiAiLines": "AI-attributed lines",
  "overview.kpiReviewed": "AI lines reviewed",
  "overview.reposByGap": "Repositories by review gap",
  "overview.noRepos": "No repositories.",
  "overview.recent": "Recent activity",
  "overview.noRecent": "No recent events.",
  "overview.unreviewed": "{n} unreviewed",
  "overview.reviewed": "reviewed",
  "overview.noAi": "no AI lines",

  // Repositories
  "repos.title": "Repositories",
  "repos.empty": "No repositories yet.",
  "repos.colRepo": "Repository",
  "repos.colEvents": "Events",

  // Repo detail
  "repoDetail.events": "{n} events",
  "repoDetail.attributedFiles": "{n} attributed files",
  "repoDetail.lastActivity": "last activity {time}",
  "repoDetail.noActivity": "no activity",
  "repoDetail.kpiAiShare": "AI-attributed lines",
  "repoDetail.kpiReviewed": "AI lines reviewed",
  "repoDetail.kpiAiLines": "AI lines",
  "repoDetail.attributedFilesTitle": "Attributed files",
  "repoDetail.noAttribution": "No attribution recorded yet.",
  "repoDetail.ranges": "{n} ranges",
  "repoDetail.contributors": "Contributors",
  "repoDetail.noContributors": "None recorded.",

  // Source connection (admin)
  "source.title": "Source connection",
  "source.none": "Not connected. Link this repo to its provider to deep-link and view source in the dashboard.",
  "source.connectedPublic": "Connected — public source.",
  "source.connectedPrivate": "Connected — private source, proxied with a stored token.",
  "source.connect": "Connect source",
  "source.edit": "Edit",
  "source.disconnect": "Disconnect",
  "source.provider": "Provider",
  "source.slug": "Repository (owner/name)",
  "source.branch": "Branch",
  "source.privateToggle": "Private repository",
  "source.token": "Access token",
  "source.tokenKeep": "Leave blank to keep the current token.",
  "source.tokenHelp": "Read-only token, scoped to this repo. Stored on the hub and never shown again.",
  "source.privateGithubOnly": "Guided private setup is GitHub-only. For GitLab/Bitbucket, use Advanced below.",
  "source.advanced": "Advanced — edit templates directly",
  "source.linkTmpl": "Link template",
  "source.rawTmpl": "Raw template",
  "source.save": "Save",
  "source.cancel": "Cancel",
  "source.slugRequired": "Enter the repository as owner/name.",
  "source.preview": "Preview",

  // File view
  "fileView.empty": "No attribution recorded for this file.",
  "fileView.blob": "blob",
  "fileView.attributedRanges": "{n} attributed ranges",
  "fileView.note": "Provenance metadata only — the hub stores no source text.",
  "fileView.noteLinks": "Source links open the lines at your configured provider.",
  "fileView.showSource": "Show source",
  "fileView.hideSource": "Hide source",
  "fileView.fetchedNote": "Fetched in your browser straight from the provider — never via the hub.",
  "fileView.fetchedNoteProxy": "Fetched through the hub from your connected private provider.",
  "fileView.loadError":
    "Couldn't load source: {err}. The provider may require auth or block cross-origin reads — use the per-range links instead.",
  "fileView.noRawUrl": "no raw source URL configured",
  "fileView.tooLarge": "file too large to inline",
  "fileView.rangeHead": "{origin} · lines {start}–{end}",
  "fileView.colLines": "Lines",
  "fileView.colOrigin": "Origin",
  "fileView.colAgent": "Agent / model",
  "fileView.colConf": "Conf.",
  "fileView.colReviewed": "Reviewed",
  "fileView.colSource": "Source",
  "fileView.view": "View ↗",

  // Sessions
  "sessions.title": "Sessions",
  "sessions.empty": "No sessions yet.",
  "sessions.colSession": "Session",
  "sessions.colActors": "Actors",
  "sessions.colEvents": "Events",
  "sessions.colLastActivity": "Last activity",

  // Session detail
  "sessionDetail.events": "{n} events",
  "sessionDetail.truncated": "showing the first {n} (truncated)",

  // Audit
  "audit.title": "Audit log",
  "audit.chainOk": "Chain verified",
  "audit.chainOkTitle": "The tamper-evident hash chain verifies",
  "audit.chainBad": "Chain broken",
  "audit.chainBadTitle": "The audit hash chain failed verification",
  "audit.filterActor": "Actor (member id)",
  "audit.filterAction": "Action (e.g. policy.update)",
  "audit.range7": "Last 7 days",
  "audit.range30": "Last 30 days",
  "audit.range90": "Last 90 days",
  "audit.range365": "Last year",
  "audit.apply": "Apply",
  "audit.none": "No audit entries match.",
  "audit.colWhen": "When",
  "audit.colActor": "Actor",
  "audit.colAction": "Action",
  "audit.colDetail": "Detail",
  "audit.colHash": "Entry hash",
  "audit.loadMore": "Load more",

  // Exports
  "exports.title": "Exports",
  "exports.intro":
    "Generate a portable snapshot of your org's activity log, tamper-evident audit trail, or a full compliance evidence pack (every repo's SLSA provenance + latest policy compliance + audit-chain status). Exports run as background jobs; large orgs may take a moment.",
  "exports.evidence": "Evidence pack",
  "exports.events": "Export events",
  "exports.audit": "Export audit log",
  "exports.queued": "Queued {kind} export.",
  "exports.empty": "No exports yet.",
  "exports.colKind": "Kind",
  "exports.colStatus": "Status",
  "exports.colCreated": "Created",
  "exports.colUpdated": "Updated",
  "exports.download": "Download",
  "exports.preparing": "Preparing…",
  "exports.error": "error",

  // Policies
  "policies.title": "Policy compliance",
  "policies.subPre": "Your",
  "policies.subPost": "policy evaluated against every repo's recorded attribution.",
  "policies.lastRun": "Last run {time}.",
  "policies.reevaluate": "Re-evaluate",
  "policies.evaluating": "Evaluating…",
  "policies.emptyTitle": "No evaluation yet",
  "policies.emptyBodyPre": "Upload a policy named",
  "policies.emptyBodyPost":
    "(via PUT /v1/orgs/{org}/policies/default or the admin CLI), then run an evaluation to see per-repo compliance here.",
  "policies.runEval": "Run evaluation",
  "policies.kpiRepos": "Repos evaluated",
  "policies.kpiViolations": "Open violations",
  "policies.kpiBySeverity": "By severity",
  "policies.sevHigh": "{n} high",
  "policies.sevMed": "{n} med",
  "policies.sevLow": "{n} low",
  "policies.colRepo": "Repository",
  "policies.colAiRanges": "AI ranges",
  "policies.colViolations": "Violations",
  "policies.colSeverity": "Severity",
  "policies.colPolicy": "Policy",
  "policies.colEvaluated": "Evaluated",
  "policies.compliant": "Compliant",

  // People & Access
  "people.title": "People & Access",
  "people.ssoTitle": "Single sign-on",
  "people.enabled": "Enabled",
  "people.notConfigured": "Not configured",
  "people.oidcNotSetup": "OIDC is not set up on this hub.",
  "people.scimTitle": "SCIM provisioning",
  "people.scimActive": "Active",
  "people.tokenIssued": "Token issued",
  "people.noScimToken": "No SCIM token has been minted.",
  "people.members": "Members",
  "people.groups": "Groups",
  "people.boundToSso": "{n} bound to SSO",
  "people.scimManaged": "SCIM-managed",
  "people.noMembers": "No members yet.",
  "people.colMember": "Member",
  "people.colRole": "Role",
  "people.colEmail": "Email",
  "people.colSso": "SSO",
  "people.colStatus": "Status",
  "people.bound": "Bound",
  "people.boundTitle": "Bound to an SSO identity",
  "people.statusActive": "Active",
  "people.statusDeactivated": "Deactivated",
  "people.noGroupsPre": "No SCIM groups. Groups named",
  "people.noGroupsPost": "drive member roles automatically.",
  "people.colGroup": "Group",
  "people.colMapsToRole": "Maps to role",
  "people.colMembers": "Members",
  "people.informational": "informational",
};

const nl: Record<string, string> = {
  // Common
  "common.retry": "Opnieuw",
  "common.loading": "Laden…",
  "common.by": "door {actor}",
  "common.ai": "AI",
  "common.human": "mens",
  "common.mixed": "gemengd",

  // App shell / nav
  "nav.overview": "Overzicht",
  "nav.repos": "Repository's",
  "nav.sessions": "Sessies",
  "nav.policies": "Beleid",
  "nav.people": "Mensen & Toegang",
  "nav.exports": "Exports",
  "nav.audit": "Auditlog",
  "nav.soon": "binnenkort",
  "shell.skip": "Naar inhoud",
  "shell.search": "Zoeken",
  "shell.signOut": "Uitloggen",
  "shell.org": "Organisatie",
  "shell.role": "Jouw rol",
  "shell.densityTitle": "Weergavedichtheid",
  "shell.themeTitle": "Thema",
  "shell.langTitle": "Taal",
  "density.comfortable": "Ruim",
  "density.compact": "Compact",
  "theme.system": "Auto",
  "theme.light": "Licht",
  "theme.dark": "Donker",

  // Command palette
  "palette.label": "Commandopalet",
  "palette.placeholder": "Spring naar…",
  "palette.search": "Zoek commando's",
  "palette.none": "Geen resultaten",
  "palette.navigate": "navigeren",
  "palette.open": "openen",
  "palette.close": "sluiten",
  "hint.compliance": "naleving",
  "hint.admin": "beheer",

  // App-level
  "app.notFound": "Niet gevonden",
  "app.backToOverview": "Terug naar overzicht",
  "app.failed": "laden mislukt",

  // Trend
  "trend.activity": "Activiteit ({days} dagen)",
  "trend.noActivity": "Geen activiteit in dit bereik.",

  // Overview
  "overview.title": "Overzicht",
  "overview.emptyTitle": "Nog geen activiteit voor deze organisatie.",
  "overview.emptyHintPre": "Koppel een repo en push provenance, bijv.",
  "overview.emptyHintPost": "of de hub-ingest-API.",
  "overview.kpiEvents": "Events",
  "overview.kpiSessions": "Sessies",
  "overview.kpiRepos": "Repository's",
  "overview.kpiAiLines": "AI-toegeschreven regels",
  "overview.kpiReviewed": "AI-regels gereviewd",
  "overview.reposByGap": "Repository's op review-achterstand",
  "overview.noRepos": "Geen repository's.",
  "overview.recent": "Recente activiteit",
  "overview.noRecent": "Geen recente events.",
  "overview.unreviewed": "{n} ongereviewd",
  "overview.reviewed": "gereviewd",
  "overview.noAi": "geen AI-regels",

  // Repositories
  "repos.title": "Repository's",
  "repos.empty": "Nog geen repository's.",
  "repos.colRepo": "Repository",
  "repos.colEvents": "Events",

  // Repo detail
  "repoDetail.events": "{n} events",
  "repoDetail.attributedFiles": "{n} toegeschreven bestanden",
  "repoDetail.lastActivity": "laatste activiteit {time}",
  "repoDetail.noActivity": "geen activiteit",
  "repoDetail.kpiAiShare": "AI-toegeschreven regels",
  "repoDetail.kpiReviewed": "AI-regels gereviewd",
  "repoDetail.kpiAiLines": "AI-regels",
  "repoDetail.attributedFilesTitle": "Toegeschreven bestanden",
  "repoDetail.noAttribution": "Nog geen attributie vastgelegd.",
  "repoDetail.ranges": "{n} bereiken",
  "repoDetail.contributors": "Bijdragers",
  "repoDetail.noContributors": "Geen vastgelegd.",

  // Source connection (admin)
  "source.title": "Bronkoppeling",
  "source.none": "Niet gekoppeld. Koppel deze repo aan z'n provider om broncode in het dashboard te deep-linken en te bekijken.",
  "source.connectedPublic": "Gekoppeld — publieke broncode.",
  "source.connectedPrivate": "Gekoppeld — private broncode, geproxyt met een opgeslagen token.",
  "source.connect": "Bron koppelen",
  "source.edit": "Bewerken",
  "source.disconnect": "Ontkoppelen",
  "source.provider": "Provider",
  "source.slug": "Repository (owner/naam)",
  "source.branch": "Branch",
  "source.privateToggle": "Private repository",
  "source.token": "Toegangstoken",
  "source.tokenKeep": "Laat leeg om het huidige token te behouden.",
  "source.tokenHelp": "Alleen-lezen token, beperkt tot deze repo. Op de hub opgeslagen en nooit meer getoond.",
  "source.privateGithubOnly": "Begeleide private-setup is alleen voor GitHub. Gebruik voor GitLab/Bitbucket de Geavanceerd-sectie hieronder.",
  "source.advanced": "Geavanceerd — templates direct bewerken",
  "source.linkTmpl": "Link-template",
  "source.rawTmpl": "Raw-template",
  "source.save": "Opslaan",
  "source.cancel": "Annuleren",
  "source.slugRequired": "Voer de repository in als owner/naam.",
  "source.preview": "Voorbeeld",

  // File view
  "fileView.empty": "Geen attributie vastgelegd voor dit bestand.",
  "fileView.blob": "blob",
  "fileView.attributedRanges": "{n} toegeschreven bereiken",
  "fileView.note": "Alleen provenance-metadata — de hub bewaart geen broncode.",
  "fileView.noteLinks": "Bronlinks openen de regels bij je geconfigureerde provider.",
  "fileView.showSource": "Bron tonen",
  "fileView.hideSource": "Bron verbergen",
  "fileView.fetchedNote": "In je browser rechtstreeks bij de provider opgehaald — nooit via de hub.",
  "fileView.fetchedNoteProxy": "Via de hub opgehaald bij je gekoppelde private provider.",
  "fileView.loadError":
    "Bron laden mislukt: {err}. De provider vereist mogelijk auth of blokkeert cross-origin reads — gebruik anders de links per bereik.",
  "fileView.noRawUrl": "geen raw-bron-URL geconfigureerd",
  "fileView.tooLarge": "bestand te groot om inline te tonen",
  "fileView.rangeHead": "{origin} · regels {start}–{end}",
  "fileView.colLines": "Regels",
  "fileView.colOrigin": "Oorsprong",
  "fileView.colAgent": "Agent / model",
  "fileView.colConf": "Conf.",
  "fileView.colReviewed": "Gereviewd",
  "fileView.colSource": "Bron",
  "fileView.view": "Bekijk ↗",

  // Sessions
  "sessions.title": "Sessies",
  "sessions.empty": "Nog geen sessies.",
  "sessions.colSession": "Sessie",
  "sessions.colActors": "Actoren",
  "sessions.colEvents": "Events",
  "sessions.colLastActivity": "Laatste activiteit",

  // Session detail
  "sessionDetail.events": "{n} events",
  "sessionDetail.truncated": "eerste {n} getoond (afgekapt)",

  // Audit
  "audit.title": "Auditlog",
  "audit.chainOk": "Keten geverifieerd",
  "audit.chainOkTitle": "De tamper-evident hashketen verifieert",
  "audit.chainBad": "Keten verbroken",
  "audit.chainBadTitle": "Verificatie van de audit-hashketen is mislukt",
  "audit.filterActor": "Actor (member-id)",
  "audit.filterAction": "Actie (bijv. policy.update)",
  "audit.range7": "Laatste 7 dagen",
  "audit.range30": "Laatste 30 dagen",
  "audit.range90": "Laatste 90 dagen",
  "audit.range365": "Laatste jaar",
  "audit.apply": "Toepassen",
  "audit.none": "Geen audit-items gevonden.",
  "audit.colWhen": "Wanneer",
  "audit.colActor": "Actor",
  "audit.colAction": "Actie",
  "audit.colDetail": "Detail",
  "audit.colHash": "Entry-hash",
  "audit.loadMore": "Meer laden",

  // Exports
  "exports.title": "Exports",
  "exports.intro":
    "Genereer een draagbare snapshot van het activiteitenlog, de tamper-evident audit-trail, of een volledig compliance-evidence-pakket (SLSA-provenance per repo + laatste beleidsnaleving + audit-keten-status). Exports draaien als achtergrondtaken; grote organisaties kunnen even duren.",
  "exports.evidence": "Evidence-pakket",
  "exports.events": "Events exporteren",
  "exports.audit": "Auditlog exporteren",
  "exports.queued": "{kind}-export in wachtrij gezet.",
  "exports.empty": "Nog geen exports.",
  "exports.colKind": "Soort",
  "exports.colStatus": "Status",
  "exports.colCreated": "Aangemaakt",
  "exports.colUpdated": "Bijgewerkt",
  "exports.download": "Downloaden",
  "exports.preparing": "Voorbereiden…",
  "exports.error": "fout",

  // Policies
  "policies.title": "Beleidsnaleving",
  "policies.subPre": "Je",
  "policies.subPost": "beleid geëvalueerd tegen de vastgelegde attributie van elke repo.",
  "policies.lastRun": "Laatste run {time}.",
  "policies.reevaluate": "Opnieuw evalueren",
  "policies.evaluating": "Evalueren…",
  "policies.emptyTitle": "Nog geen evaluatie",
  "policies.emptyBodyPre": "Upload een beleid met de naam",
  "policies.emptyBodyPost":
    "(via PUT /v1/orgs/{org}/policies/default of de admin-CLI) en draai een evaluatie om hier naleving per repo te zien.",
  "policies.runEval": "Evaluatie draaien",
  "policies.kpiRepos": "Repo's geëvalueerd",
  "policies.kpiViolations": "Open overtredingen",
  "policies.kpiBySeverity": "Per ernst",
  "policies.sevHigh": "{n} hoog",
  "policies.sevMed": "{n} mid",
  "policies.sevLow": "{n} laag",
  "policies.colRepo": "Repository",
  "policies.colAiRanges": "AI-bereiken",
  "policies.colViolations": "Overtredingen",
  "policies.colSeverity": "Ernst",
  "policies.colPolicy": "Beleid",
  "policies.colEvaluated": "Geëvalueerd",
  "policies.compliant": "Compliant",

  // People & Access
  "people.title": "Mensen & Toegang",
  "people.ssoTitle": "Single sign-on",
  "people.enabled": "Ingeschakeld",
  "people.notConfigured": "Niet geconfigureerd",
  "people.oidcNotSetup": "OIDC is niet ingesteld op deze hub.",
  "people.scimTitle": "SCIM-provisioning",
  "people.scimActive": "Actief",
  "people.tokenIssued": "Token uitgegeven",
  "people.noScimToken": "Er is geen SCIM-token aangemaakt.",
  "people.members": "Leden",
  "people.groups": "Groepen",
  "people.boundToSso": "{n} gekoppeld aan SSO",
  "people.scimManaged": "SCIM-beheerd",
  "people.noMembers": "Nog geen leden.",
  "people.colMember": "Lid",
  "people.colRole": "Rol",
  "people.colEmail": "E-mail",
  "people.colSso": "SSO",
  "people.colStatus": "Status",
  "people.bound": "Gekoppeld",
  "people.boundTitle": "Gekoppeld aan een SSO-identiteit",
  "people.statusActive": "Actief",
  "people.statusDeactivated": "Gedeactiveerd",
  "people.noGroupsPre": "Geen SCIM-groepen. Groepen met de naam",
  "people.noGroupsPost": "sturen automatisch ledenrollen aan.",
  "people.colGroup": "Groep",
  "people.colMapsToRole": "Wordt rol",
  "people.colMembers": "Leden",
  "people.informational": "informatief",
};

const CATALOG: Record<Locale, Record<string, string>> = { en, nl };

/** The message keys defined for a locale (used by the parity test). */
export function localeKeys(locale: Locale): string[] {
  return Object.keys(CATALOG[locale]);
}

/** Validate an arbitrary stored value into a known locale (defaults to en). */
export function normalizeLocale(raw: string | null): Locale {
  return LOCALES.includes(raw as Locale) ? (raw as Locale) : "en";
}

/** The next locale in the cycle (en → nl → en). */
export function nextLocale(l: Locale): Locale {
  return LOCALES[(LOCALES.indexOf(l) + 1) % LOCALES.length]!;
}

/**
 * Resolve `key` in `locale`, interpolating `{var}` placeholders. Falls back to
 * English when the key is missing in `locale`, and returns the key itself when
 * it's unknown in every locale.
 */
export function translate(locale: Locale, key: string, vars?: Vars): string {
  const template = CATALOG[locale][key] ?? CATALOG.en[key] ?? key;
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (_m, name: string) =>
    name in vars ? String(vars[name]) : `{${name}}`,
  );
}

/** Read the saved locale preference. */
export function loadLocale(): Locale {
  try {
    return normalizeLocale(localStorage.getItem(KEY));
  } catch {
    return "en";
  }
}

/** Persist + reflect a locale on the document. */
export function saveLocale(l: Locale): void {
  try {
    localStorage.setItem(KEY, l);
  } catch {
    /* storage unavailable — locale still applies for this session */
  }
  if (typeof document !== "undefined") document.documentElement.lang = l;
}
