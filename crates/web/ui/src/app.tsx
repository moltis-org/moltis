// ── Entry point ────────────────────────────────────────────

import { render } from "preact";
import prettyBytes from "pretty-bytes";
import { applyIdentityFavicon, formatPageTitle } from "./branding";
import * as _chatUi from "./chat-ui";
import * as _codeHighlight from "./code-highlight";
import { initHighlighter } from "./code-highlight";
import { SessionList } from "./components/SessionList";
import * as _events from "./events";
import { onEvent } from "./events";
import * as gon from "./gon";
import "./github-stats";
import * as _helpers from "./helpers";
import * as _i18n from "./i18n";
import { init as initI18n, translateStaticElements } from "./i18n";
import { initMobile, toggleSessions } from "./mobile";
import { fetchModels } from "./models";
import { updateNavCounts } from "./nav-counts";
import * as _nodeSelector from "./nodes-selector";
import * as _channelsPage from "./pages/ChannelsPage";
import { renderSessionProjectSelect } from "./project-combo";
import { fetchProjects, renderProjectSelect } from "./projects";
import * as _providers from "./providers";
import { initPWA } from "./pwa";
import { initInstallBanner } from "./pwa-install";
import { mount, navigate, registerPage, sessionPath } from "./router";
import { routes } from "./routes";
import { updateSandboxImageUI, updateSandboxUI } from "./sandbox";
import * as _sessions from "./sessions";
import { fetchSessions, refreshWelcomeCardIfNeeded, removeSessionFromClientState, renderSessionList } from "./sessions";
import * as S from "./state";
import * as modelStore from "./stores/model-store";
import * as _modelStore from "./stores/model-store";
import * as _nodeStore from "./stores/node-store";
import * as projectStore from "./stores/project-store";
import * as _sessionHistoryCache from "./stores/session-history-cache";
import * as _sessionStoreModule from "./stores/session-store";
import { insertSessionInOrder, sessionStore } from "./stores/session-store";
import { initTheme, injectMarkdownStyles } from "./theme";
import { GlobalDialogs } from "./ui";
import { connect } from "./websocket";
import * as _wsConnect from "./ws-connect";

// Expose stores and modules on window for E2E test shims.
// The shim files in assets/js/ proxy to these bundled modules.
window.__moltis_stores = { sessionStore, modelStore, projectStore };
window.__moltis_state = S;
window.__moltis_modules = {
	state: S,
	helpers: _helpers,
	events: _events,
	"chat-ui": _chatUi,
	sessions: _sessions,
	gon,
	"code-highlight": _codeHighlight,
	"ws-connect": _wsConnect,
	"nodes-selector": _nodeSelector,
	providers: _providers,
	"page-channels": _channelsPage,
	i18n: _i18n,
	"stores/model-store": _modelStore,
	"stores/session-store": _sessionStoreModule,
	"stores/node-store": _nodeStore,
	"stores/session-history-cache": _sessionHistoryCache,
};

// Import page modules to register their routes
import "./pages/ChatPage";
import "./pages/CronsPage";
import "./pages/ProjectsPage";
import "./pages/SkillsPage";
import "./pages/MetricsPage";
import "./pages/SettingsPage"; // also imports channels, providers, mcp, hooks, images, logs

// Import side-effect modules
import "./nav-counts";
import "./session-search";
import "./time-format";

// ── Types ────────────────────────────────────────────────────

interface MemInfo {
	process: number;
	available: number;
	total: number;
	localLlamaCpp?: number;
	local_llama_cpp?: number;
}

interface IdentityInfo {
	emoji?: string;
	name?: string;
	user_name?: string;
	[key: string]: unknown;
}

interface AuthStatus {
	authenticated?: boolean;
	auth_disabled?: boolean;
	has_password?: boolean;
	has_passkeys?: boolean;
	localhost_only?: boolean;
	setup_required?: boolean;
}

interface BootstrapData {
	channels?: { channels?: unknown[] } | unknown[];
	sessions?: unknown[];
	models?: ModelEntry[];
	projects?: ProjectEntry[];
	sandbox?: unknown;
	counts?: Record<string, number>;
}

interface ModelEntry {
	id: string;
	[key: string]: unknown;
}

interface ProjectEntry {
	id: string;
	[key: string]: unknown;
}

interface SessionEntry {
	key: string;
	[key: string]: unknown;
}

// ── Helpers ──────────────────────────────────────────────────

function preferredChatPath(): string {
	const key = localStorage.getItem("moltis-session") || "main";
	return sessionPath(key);
}

// Redirect root to the active/default chat session.
registerPage("/", () => {
	const path = preferredChatPath();
	if (location.pathname !== path) {
		history.replaceState(null, "", path);
	}
	mount(path);
});

initTheme();
injectMarkdownStyles();
initHighlighter();
initPWA();
initMobile();
const i18nReady = initI18n()
	.then(() => {
		translateStaticElements(document.documentElement);
	})
	.catch((err: unknown) => {
		console.warn("[i18n] failed to initialize", err);
	});
let appStarted = false;

function startAppAfterI18n(): void {
	if (appStarted) return;
	appStarted = true;
	i18nReady.finally(() => {
		startApp();
	});
}

const UPDATE_DISMISS_KEY = "moltis-update-dismissed-version";
let currentUpdateVersion: string | null = null;

// Apply server-injected identity immediately (no async wait), and
// keep the header in sync whenever gon.identity is refreshed.
try {
	applyIdentity(gon.get("identity") as IdentityInfo | null);
} catch (_) {
	// Non-fatal — page still works without identity in the header.
}
gon.onChange("identity", applyIdentity as (v: unknown) => void);

// Show git branch banner when running on a non-main branch.
try {
	showBranchBanner(gon.get("git_branch") as string | null);
} catch (_) {
	// Non-fatal — branch indicator is cosmetic.
}
gon.onChange("git_branch", showBranchBanner as (v: unknown) => void);
try {
	showUpdateBanner(gon.get("update") as { available?: boolean; latest_version?: string; release_url?: string } | null);
} catch (_) {
	// Non-fatal — update indicator is cosmetic.
}
gon.onChange("update", showUpdateBanner as (v: unknown) => void);
onEvent("update.available", showUpdateBanner as (payload: unknown) => void);
initUpdateBannerDismiss();
showVaultBanner(gon.get("vault_status") as string | null);
gon.onChange("vault_status", showVaultBanner as (v: unknown) => void);

function upsertSessionFromEvent(entry: SessionEntry | null): boolean {
	if (!entry?.key) return false;
	sessionStore.upsert(entry as never);
	const legacy = S.sessions.slice() as SessionEntry[];
	const idx = legacy.findIndex((session) => session.key === entry.key);
	let nextEntry = { ...entry };
	if (idx >= 0) {
		nextEntry = { ...legacy[idx], ...entry };
	}
	S.setSessions(insertSessionInOrder(legacy as never[], nextEntry as never));
	renderSessionList();
	return true;
}

function removeSessionFromEvent(sessionKey: string): boolean {
	return removeSessionFromClientState(sessionKey, { navigateIfActive: true });
}

onEvent("session", (_payload: unknown) => {
	const payload = _payload as Record<string, unknown>;
	if (!payload?.kind) return;
	if (payload.kind === "deleted") {
		if (!removeSessionFromEvent(payload.sessionKey as string)) {
			fetchSessions();
		}
		return;
	}
	if (upsertSessionFromEvent((payload.entry as SessionEntry) || null)) return;
	if (payload.kind === "created" || payload.kind === "patched") {
		fetchSessions();
	}
});

function seedSessionsFromGon(): void {
	const seeded = gon.get("sessions_recent") as SessionEntry[] | null;
	if (!Array.isArray(seeded) || seeded.length === 0) return;
	if (sessionStore.sessions.value.length > 0) return;
	sessionStore.setAll(seeded as never[]);
	S.setSessions(seeded as never[]);
	renderSessionList();
}

seedSessionsFromGon();

function applyMemory(mem: MemInfo | null): void {
	if (!mem) return;
	const el = document.getElementById("memoryInfo");
	if (!el) return;
	const fmt = (b: number): string => prettyBytes(b, { maximumFractionDigits: 0, space: false });
	let localLlamaCpp = 0;
	if (typeof mem.localLlamaCpp === "number") {
		localLlamaCpp = mem.localLlamaCpp;
	} else if (typeof mem.local_llama_cpp === "number") {
		// Backward compatibility with older payload casing.
		localLlamaCpp = mem.local_llama_cpp;
	}
	const localLlamaPart = localLlamaCpp > 0 ? ` (llama.cpp ${fmt(localLlamaCpp)})` : "";
	el.textContent = `${fmt(mem.process)}${localLlamaPart} \u00b7 ${fmt(mem.available)} free / ${fmt(mem.total)}`;
}

applyMemory(gon.get("mem") as MemInfo | null);
gon.onChange("mem", applyMemory as (v: unknown) => void);
onEvent("tick", (_payload: unknown) => {
	const payload = _payload as Record<string, unknown>;
	applyMemory(payload.mem as MemInfo | null);
});

// Logout button — wire up click handler once.
const logoutBtn = document.getElementById("logoutBtn");
const settingsBtn = document.getElementById("settingsBtn");
const mobileMenuBtn = document.getElementById("mobileMenuBtn");
const mobileMenuOverlay = document.getElementById("mobileMenuOverlay");
const mobileMenuPanel = document.getElementById("mobileMenuPanel");
const mobileMenuSessionsBtn = document.getElementById("mobileMenuSessionsBtn");
const mobileMenuSettingsBtn = document.getElementById("mobileMenuSettingsBtn");
const mobileMenuLogoutBtn = document.getElementById("mobileMenuLogoutBtn");

function closeMobileMenu(): void {
	if (mobileMenuPanel) mobileMenuPanel.classList.remove("open");
	if (mobileMenuOverlay) mobileMenuOverlay.classList.remove("visible");
}

function toggleMobileMenu(): void {
	if (!(mobileMenuPanel && mobileMenuOverlay)) return;
	const isOpen = mobileMenuPanel.classList.contains("open");
	if (isOpen) {
		closeMobileMenu();
		return;
	}
	mobileMenuPanel.classList.add("open");
	mobileMenuOverlay.classList.add("visible");
}

function performLogout(): void {
	closeMobileMenu();
	fetch("/api/auth/logout", { method: "POST" }).finally(() => {
		location.href = "/";
	});
}

if (logoutBtn) {
	logoutBtn.addEventListener("click", performLogout);
}
if (settingsBtn) {
	settingsBtn.addEventListener("click", () => {
		closeMobileMenu();
		navigate(routes.identity as string);
	});
}
if (mobileMenuBtn) {
	mobileMenuBtn.addEventListener("click", (e) => {
		e.stopPropagation();
		toggleMobileMenu();
	});
}
if (mobileMenuOverlay) {
	mobileMenuOverlay.addEventListener("click", closeMobileMenu);
}
if (mobileMenuSettingsBtn) {
	mobileMenuSettingsBtn.addEventListener("click", () => {
		closeMobileMenu();
		navigate(routes.identity as string);
	});
}
if (mobileMenuSessionsBtn) {
	mobileMenuSessionsBtn.addEventListener("click", () => {
		closeMobileMenu();
		if (window.innerWidth < 768) {
			toggleSessions();
		}
	});
}
if (mobileMenuLogoutBtn) {
	mobileMenuLogoutBtn.addEventListener("click", performLogout);
}
window.addEventListener("resize", () => {
	if (window.innerWidth >= 768) closeMobileMenu();
});
document.addEventListener("keydown", (e) => {
	if (e.key === "Escape") closeMobileMenu();
});
document.addEventListener("click", (e) => {
	if (!mobileMenuPanel?.classList.contains("open")) return;
	if (mobileMenuPanel.contains(e.target as Node)) return;
	if (mobileMenuBtn?.contains(e.target as Node)) return;
	closeMobileMenu();
});

function updateAuthChrome(auth: AuthStatus | null): void {
	const showLogout = !!(auth?.authenticated && !auth.auth_disabled && (auth.has_password || auth.has_passkeys));
	if (logoutBtn) {
		logoutBtn.style.display = showLogout ? "" : "none";
	}
	if (mobileMenuLogoutBtn) {
		mobileMenuLogoutBtn.style.display = showLogout ? "" : "none";
	}
	if (mobileMenuPanel) {
		mobileMenuPanel.classList.toggle("logout-hidden", !showLogout);
	}
	if (!showLogout) {
		closeMobileMenu();
	}

	const banner = document.getElementById("authDisabledBanner");
	if (banner) {
		const showAuthDisabled = !!(auth?.auth_disabled && !auth.localhost_only);
		banner.style.display = showAuthDisabled ? "" : "none";
	}
}

function refreshAuthChrome(): Promise<AuthStatus | null> {
	return fetch("/api/auth/status")
		.then((r) => (r.ok ? r.json() : null))
		.then((auth: AuthStatus | null) => {
			updateAuthChrome(auth);
			return auth;
		})
		.catch(() => null);
}

window.addEventListener("moltis:auth-status-changed", () => {
	refreshAuthChrome()
		.then((auth) => {
			if (!auth) return;
			if (auth.setup_required) {
				clearSensitiveData();
				window.location.assign("/onboarding");
				return;
			}
			if (!auth.authenticated) {
				clearSensitiveData();
				window.location.assign("/login");
			}
		})
		.finally(() => {
			window.dispatchEvent(new CustomEvent("moltis:auth-status-sync-complete"));
		});
});

/**
 * Purge cached sensitive data so that a logged-out page cannot display
 * session previews, identity info, or other user-scoped state.
 */
function clearSensitiveData(): void {
	// Clear session store and legacy state
	sessionStore.setAll([]);
	S.setSessions([]);
	renderSessionList();

	// Clear model and project stores
	modelStore.setAll([]);
	S.setModels([]);
	projectStore.setAll([]);
	S.setProjects([]);

	// Clear identity from gon so sidebar/header no longer shows it
	gon.set("identity", null);
	gon.set("sessions_recent", null);
	// Signal vault sealed so SessionList shows the correct placeholder
	gon.set("vault_status", "sealed");
}

// Seed sandbox info from gon so the settings page can render immediately
// without waiting for the auth-protected /api/bootstrap fetch.
try {
	const gonSandbox = gon.get("sandbox");
	if (gonSandbox) S.setSandboxInfo(gonSandbox);
} catch (_) {
	// Non-fatal — sandbox info will arrive via bootstrap.
}
// Check auth status before mounting the app.
fetch("/api/auth/status")
	.then((r) => (r.ok ? r.json() : null))
	.then((auth: AuthStatus | null) => {
		if (!auth) {
			// Auth endpoints not available — no auth configured, proceed normally.
			startAppAfterI18n();
			return;
		}
		if (auth.setup_required) {
			window.location.assign("/onboarding");
			return;
		}
		if (!auth.authenticated) {
			// Server-side middleware handles the redirect to /login.
			// This is a defense-in-depth fallback for edge cases
			// (e.g. session expired after the page was already served).
			window.location.assign("/login");
			return;
		}
		updateAuthChrome(auth);
		startAppAfterI18n();
	})
	.catch(() => {
		// If auth check fails, proceed anyway (backward compat).
		startAppAfterI18n();
	});

function showUpdateBanner(update: { available?: boolean; latest_version?: string; release_url?: string } | null): void {
	const el = document.getElementById("updateBanner");
	if (!el) return;

	const latestVersion = update?.latest_version || null;
	currentUpdateVersion = latestVersion;
	const dismissedVersion = localStorage.getItem(UPDATE_DISMISS_KEY);

	if (update?.available && (!latestVersion || dismissedVersion !== latestVersion)) {
		const versionEl = document.getElementById("updateLatestVersion");
		if (versionEl) {
			versionEl.textContent = latestVersion ? `v${latestVersion}` : "";
		}
		const linkEl = S.$<HTMLAnchorElement>("updateReleaseLink");
		if (linkEl && update.release_url) {
			linkEl.href = update.release_url;
		}
		el.style.display = "";
	} else {
		el.style.display = "none";
	}
}

function initUpdateBannerDismiss(): void {
	const dismissBtn = S.$("updateDismissBtn");
	if (!dismissBtn || dismissBtn.dataset.bound === "1") return;
	dismissBtn.dataset.bound = "1";
	dismissBtn.addEventListener("click", () => {
		if (currentUpdateVersion) {
			localStorage.setItem(UPDATE_DISMISS_KEY, currentUpdateVersion);
		}
		const el = document.getElementById("updateBanner");
		if (el) el.style.display = "none";
	});
}

function showVaultBanner(status: string | null): void {
	const el = document.getElementById("vaultBanner");
	if (!el) return;
	el.style.display = status === "sealed" ? "" : "none";
}

function showBranchBanner(branch: string | null): void {
	const el = document.getElementById("branchBanner");
	if (!el) return;

	if (branch) {
		const branchNameEl = document.getElementById("branchName");
		if (branchNameEl) branchNameEl.textContent = branch;
		el.style.display = "";

		// Prefix page title with branch name.
		document.title = `[${branch}] ${formatPageTitle(gon.get("identity") as IdentityInfo | null)}`;
	} else {
		el.style.display = "none";

		// Restore original title
		document.title = formatPageTitle(gon.get("identity") as IdentityInfo | null);
	}
}

function applyIdentity(identity: IdentityInfo | null): void {
	const emojiEl = document.getElementById("titleEmoji");
	const nameEl = document.getElementById("titleName");
	if (emojiEl) emojiEl.textContent = identity?.emoji ? `${identity.emoji} ` : "";
	if (nameEl) nameEl.textContent = identity?.name || "moltis";
	applyIdentityFavicon(identity);
	const branch = gon.get("git_branch") as string | null;

	// Keep page title in sync with identity and branch.
	const title = formatPageTitle(identity);
	if (branch) {
		document.title = `[${branch}] ${title}`;
	} else {
		document.title = title;
	}
}

function applyModels(models: ModelEntry[]): void {
	const arr = models || [];
	modelStore.setAll(arr as never[]);
	// Dual-write to state.js for backward compat
	S.setModels(arr);
	if (arr.length === 0) return;
	const saved = localStorage.getItem("moltis-model") || "";
	const found = arr.find((m) => m.id === saved);
	if (found) {
		modelStore.select(found.id);
		S.setSelectedModelId(found.id);
	} else {
		modelStore.select(arr[0].id);
		S.setSelectedModelId(arr[0].id);
		localStorage.setItem("moltis-model", modelStore.selectedModelId.value);
	}
}

function fetchBootstrap(): void {
	// Fetch bootstrap data asynchronously — populates sidebar, models, projects
	// as soon as the data arrives, without blocking the initial page render.
	fetch("/api/bootstrap?include_sessions=false")
		.then((r) => {
			if (r.status === 401 || r.status === 403) {
				window.dispatchEvent(new CustomEvent("moltis:auth-status-changed"));
				return Promise.reject(new Error("auth"));
			}
			return r.json();
		})
		.then((boot: BootstrapData) => {
			if (boot.channels)
				S.setCachedChannels((boot.channels as { channels?: unknown[] }).channels || boot.channels || []);
			if (boot.sessions) {
				const bootSessions = boot.sessions || [];
				sessionStore.setAll(bootSessions as never[]);
				// Dual-write to state.js for backward compat
				S.setSessions(bootSessions);
				renderSessionList();
			} else {
				// Keep full list fetch separate from bootstrap payload size.
				fetchSessions();
			}
			if (boot.models) applyModels(boot.models);
			refreshWelcomeCardIfNeeded();
			if (boot.projects) {
				const bootProjects = boot.projects || [];
				projectStore.setAll(bootProjects as never[]);
				// Dual-write to state.js for backward compat
				S.setProjects(bootProjects);
				renderProjectSelect();
				renderSessionProjectSelect();
			}
			S.setSandboxInfo(boot.sandbox || null);
			// Re-apply sandbox UI now that we know the backend status.
			// This fixes the race where the chat page renders before bootstrap completes.
			updateSandboxUI(S.sessionSandboxEnabled);
			updateSandboxImageUI(S.sessionSandboxImage);
			if (boot.counts) updateNavCounts(boot.counts);
		})
		.catch(() => {
			// If bootstrap fails, hydrate from dedicated lightweight endpoints.
			fetchSessions();
			fetchModels();
			fetchProjects();
		});
}

function initSessionTabBar(): void {
	const bar = S.$("sessionTabBar");
	if (!bar) return;
	const buttons = bar.querySelectorAll<HTMLElement>(".session-tab");
	const archivedRow = S.$("archivedSessionsRow");

	function updateActive(): void {
		const current = sessionStore.sessionListTab.value;
		for (const btn of buttons) {
			btn.classList.toggle("active", btn.dataset.tab === current);
		}
		if (archivedRow) {
			archivedRow.classList.toggle("hidden", current !== "sessions");
		}
	}

	for (const btn of buttons) {
		btn.addEventListener("click", function (this: HTMLElement) {
			sessionStore.setSessionListTab(this.dataset.tab || "sessions");
			updateActive();
		});
	}
	updateActive();
}

function initArchivedSessionsToggle(): void {
	const checkbox = S.$<HTMLInputElement>("showArchivedSessions");
	if (!checkbox) return;
	checkbox.checked = sessionStore.showArchivedSessions.value;
	checkbox.addEventListener("change", function (this: HTMLInputElement) {
		sessionStore.setShowArchivedSessions(this.checked);
	});
}

function startApp(): void {
	// Mount the reactive SessionList once — signals drive all re-renders.
	const sessionListEl = S.$("sessionList");
	if (sessionListEl) render(<SessionList />, sessionListEl);

	// Mount global signal-driven dialogs (confirm, share visibility, share link).
	const dialogRoot = document.createElement("div");
	dialogRoot.id = "preactDialogRoot";
	document.body.appendChild(dialogRoot);
	render(<GlobalDialogs />, dialogRoot);

	initSessionTabBar();
	initArchivedSessionsToggle();

	let path = location.pathname;
	if (path === "/") {
		path = preferredChatPath();
		history.replaceState(null, "", path);
	}
	mount(path);
	connect();
	fetchBootstrap();
	initInstallBanner();
}
