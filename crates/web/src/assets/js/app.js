// ── Entry point ────────────────────────────────────────────

import { html } from "htm/preact";
import { render } from "preact";
import prettyBytes from "pretty-bytes";
import { applyIdentityFavicon, formatPageTitle } from "./branding.js";
import { initHighlighter } from "./code-highlight.js";
import { SessionList } from "./components/session-list.js";
import { onEvent } from "./events.js";
import * as gon from "./gon.js";
import { init as initI18n, translateStaticElements } from "./i18n.js";
import { initMobile, toggleSessions } from "./mobile.js";
import { fetchModels } from "./models.js";
import { updateNavCounts } from "./nav-counts.js";
import { renderSessionProjectSelect } from "./project-combo.js";
import { fetchProjects, renderProjectSelect } from "./projects.js";
import { initPWA } from "./pwa.js";
import { initInstallBanner } from "./pwa-install.js";
import { mount, navigate, registerPage, sessionPath } from "./router.js";
import { routes } from "./routes.js";
import { updateSandboxImageUI, updateSandboxUI } from "./sandbox.js";
import { clearSessionHistoryCache, fetchSessions, refreshWelcomeCardIfNeeded, renderSessionList } from "./sessions.js";
import * as S from "./state.js";
import { modelStore } from "./stores/model-store.js";
import { projectStore } from "./stores/project-store.js";
import { sessionStore } from "./stores/session-store.js";
import { initTheme, injectMarkdownStyles } from "./theme.js";
import { connect } from "./websocket.js";

// Expose stores on window for E2E test access.
window.__moltis_stores = { sessionStore, modelStore, projectStore };

// Import page modules to register their routes
import "./page-chat.js";
import "./page-crons.js";
import "./page-projects.js";
import "./page-skills.js";
import "./page-metrics.js";
import "./page-settings.js"; // also imports channels, providers, mcp, hooks, images, logs

// Import side-effect modules
import "./nav-counts.js";
import "./session-search.js";
import "./time-format.js";

function preferredChatPath() {
	var key = localStorage.getItem("moltis-session") || "main";
	return sessionPath(key);
}

// Redirect root to the active/default chat session.
registerPage("/", () => {
	var path = preferredChatPath();
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
var i18nReady = initI18n()
	.then(() => {
		translateStaticElements(document.documentElement);
	})
	.catch((err) => {
		console.warn("[i18n] failed to initialize", err);
	});
var appStarted = false;

function startAppAfterI18n() {
	if (appStarted) return;
	appStarted = true;
	i18nReady.finally(() => {
		startApp();
	});
}

var UPDATE_DISMISS_KEY = "moltis-update-dismissed-version";
var currentUpdateVersion = null;

// Apply server-injected identity immediately (no async wait), and
// keep the header in sync whenever gon.identity is refreshed.
try {
	applyIdentity(gon.get("identity"));
} catch (_) {
	// Non-fatal — page still works without identity in the header.
}
gon.onChange("identity", applyIdentity);

// Show git branch banner when running on a non-main branch.
try {
	showBranchBanner(gon.get("git_branch"));
} catch (_) {
	// Non-fatal — branch indicator is cosmetic.
}
gon.onChange("git_branch", showBranchBanner);
try {
	showUpdateBanner(gon.get("update"));
} catch (_) {
	// Non-fatal — update indicator is cosmetic.
}
gon.onChange("update", showUpdateBanner);
onEvent("update.available", showUpdateBanner);
initUpdateBannerDismiss();
showVaultBanner(gon.get("vault_status"));
gon.onChange("vault_status", showVaultBanner);

function upsertSessionFromEvent(entry) {
	if (!entry?.key) return false;
	sessionStore.upsert(entry);
	var legacy = S.sessions.slice();
	var idx = legacy.findIndex((session) => session.key === entry.key);
	if (idx >= 0) {
		legacy[idx] = { ...legacy[idx], ...entry };
	} else {
		legacy.push({ ...entry });
	}
	S.setSessions(legacy);
	renderSessionList();
	return true;
}

function removeSessionFromEvent(sessionKey) {
	if (!sessionKey) return false;
	var removedActive = sessionStore.activeSessionKey.value === sessionKey;
	var removed = sessionStore.remove(sessionKey);
	if (!removed) return false;
	clearSessionHistoryCache(sessionKey);
	S.setSessions(S.sessions.filter((session) => session.key !== sessionKey));
	renderSessionList();
	if (!removedActive) return true;
	var nextKey = sessionStore.activeSessionKey.value || "main";
	S.setActiveSessionKey(nextKey);
	if (location.pathname.startsWith("/chats/")) {
		navigate(sessionPath(nextKey));
	}
	return true;
}

onEvent("session", (payload) => {
	if (!payload?.kind) return;
	if (payload.kind === "deleted") {
		if (!removeSessionFromEvent(payload.sessionKey)) {
			fetchSessions();
		}
		return;
	}
	if (upsertSessionFromEvent(payload.entry || null)) return;
	if (payload.kind === "created" || payload.kind === "patched") {
		fetchSessions();
	}
});

function seedSessionsFromGon() {
	var seeded = gon.get("sessions_recent");
	if (!Array.isArray(seeded) || seeded.length === 0) return;
	if (sessionStore.sessions.value.length > 0) return;
	sessionStore.setAll(seeded);
	S.setSessions(seeded);
	renderSessionList();
}

seedSessionsFromGon();

function applyMemory(mem) {
	if (!mem) return;
	var el = document.getElementById("memoryInfo");
	if (!el) return;
	var fmt = (b) => prettyBytes(b, { maximumFractionDigits: 0, space: false });
	var localLlamaCpp = 0;
	if (typeof mem.localLlamaCpp === "number") {
		localLlamaCpp = mem.localLlamaCpp;
	} else if (typeof mem.local_llama_cpp === "number") {
		// Backward compatibility with older payload casing.
		localLlamaCpp = mem.local_llama_cpp;
	}
	var localLlamaPart = localLlamaCpp > 0 ? ` (llama.cpp ${fmt(localLlamaCpp)})` : "";
	el.textContent = `${fmt(mem.process)}${localLlamaPart} \u00b7 ${fmt(mem.available)} free / ${fmt(mem.total)}`;
}

applyMemory(gon.get("mem"));
gon.onChange("mem", applyMemory);
onEvent("tick", (payload) => applyMemory(payload.mem));

// Logout button — wire up click handler once.
var logoutBtn = document.getElementById("logoutBtn");
var settingsBtn = document.getElementById("settingsBtn");
var mobileMenuBtn = document.getElementById("mobileMenuBtn");
var mobileMenuOverlay = document.getElementById("mobileMenuOverlay");
var mobileMenuPanel = document.getElementById("mobileMenuPanel");
var mobileMenuSessionsBtn = document.getElementById("mobileMenuSessionsBtn");
var mobileMenuSettingsBtn = document.getElementById("mobileMenuSettingsBtn");
var mobileMenuLogoutBtn = document.getElementById("mobileMenuLogoutBtn");

function closeMobileMenu() {
	if (mobileMenuPanel) mobileMenuPanel.classList.remove("open");
	if (mobileMenuOverlay) mobileMenuOverlay.classList.remove("visible");
}

function toggleMobileMenu() {
	if (!(mobileMenuPanel && mobileMenuOverlay)) return;
	var isOpen = mobileMenuPanel.classList.contains("open");
	if (isOpen) {
		closeMobileMenu();
		return;
	}
	mobileMenuPanel.classList.add("open");
	mobileMenuOverlay.classList.add("visible");
}

function performLogout() {
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
		navigate(routes.identity);
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
		navigate(routes.identity);
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
	if (mobileMenuPanel.contains(e.target)) return;
	if (mobileMenuBtn?.contains(e.target)) return;
	closeMobileMenu();
});

function updateAuthChrome(auth) {
	var showLogout = !!(auth?.authenticated && !auth.auth_disabled && (auth.has_password || auth.has_passkeys));
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

	var banner = document.getElementById("authDisabledBanner");
	if (banner) {
		var showAuthDisabled = !!(auth?.auth_disabled && !auth.localhost_only);
		banner.style.display = showAuthDisabled ? "" : "none";
	}
}

function refreshAuthChrome() {
	return fetch("/api/auth/status")
		.then((r) => (r.ok ? r.json() : null))
		.then((auth) => {
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
function clearSensitiveData() {
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
	var gonSandbox = gon.get("sandbox");
	if (gonSandbox) S.setSandboxInfo(gonSandbox);
} catch (_) {
	// Non-fatal — sandbox info will arrive via bootstrap.
}
// Check auth status before mounting the app.
fetch("/api/auth/status")
	.then((r) => (r.ok ? r.json() : null))
	.then((auth) => {
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

function showUpdateBanner(update) {
	var el = document.getElementById("updateBanner");
	if (!el) return;

	var latestVersion = update?.latest_version || null;
	currentUpdateVersion = latestVersion;
	var dismissedVersion = localStorage.getItem(UPDATE_DISMISS_KEY);

	if (update?.available && (!latestVersion || dismissedVersion !== latestVersion)) {
		var versionEl = document.getElementById("updateLatestVersion");
		if (versionEl) {
			versionEl.textContent = latestVersion ? `v${latestVersion}` : "";
		}
		var linkEl = document.getElementById("updateReleaseLink");
		if (linkEl && update.release_url) {
			linkEl.href = update.release_url;
		}
		el.style.display = "";
	} else {
		el.style.display = "none";
	}
}

function initUpdateBannerDismiss() {
	var dismissBtn = document.getElementById("updateDismissBtn");
	if (!dismissBtn || dismissBtn.dataset.bound === "1") return;
	dismissBtn.dataset.bound = "1";
	dismissBtn.addEventListener("click", () => {
		if (currentUpdateVersion) {
			localStorage.setItem(UPDATE_DISMISS_KEY, currentUpdateVersion);
		}
		var el = document.getElementById("updateBanner");
		if (el) el.style.display = "none";
	});
}

function showVaultBanner(status) {
	var el = document.getElementById("vaultBanner");
	if (!el) return;
	el.style.display = status === "sealed" ? "" : "none";
}

function showBranchBanner(branch) {
	var el = document.getElementById("branchBanner");
	if (!el) return;

	if (branch) {
		document.getElementById("branchName").textContent = branch;
		el.style.display = "";

		// Prefix page title with branch name.
		document.title = `[${branch}] ${formatPageTitle(gon.get("identity"))}`;
	} else {
		el.style.display = "none";

		// Restore original title
		document.title = formatPageTitle(gon.get("identity"));
	}
}

function applyIdentity(identity) {
	var emojiEl = document.getElementById("titleEmoji");
	var nameEl = document.getElementById("titleName");
	if (emojiEl) emojiEl.textContent = identity?.emoji ? `${identity.emoji} ` : "";
	if (nameEl) nameEl.textContent = identity?.name || "moltis";
	applyIdentityFavicon(identity);
	var branch = gon.get("git_branch");

	// Keep page title in sync with identity and branch.
	var title = formatPageTitle(identity);
	if (branch) {
		document.title = `[${branch}] ${title}`;
	} else {
		document.title = title;
	}
}

function applyModels(models) {
	var arr = models || [];
	modelStore.setAll(arr);
	// Dual-write to state.js for backward compat
	S.setModels(arr);
	if (arr.length === 0) return;
	var saved = localStorage.getItem("moltis-model") || "";
	var found = arr.find((m) => m.id === saved);
	if (found) {
		modelStore.select(found.id);
		S.setSelectedModelId(found.id);
	} else {
		modelStore.select(arr[0].id);
		S.setSelectedModelId(arr[0].id);
		localStorage.setItem("moltis-model", modelStore.selectedModelId.value);
	}
}

function fetchBootstrap() {
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
		.then((boot) => {
			if (boot.channels) S.setCachedChannels(boot.channels.channels || boot.channels || []);
			if (boot.sessions) {
				var bootSessions = boot.sessions || [];
				sessionStore.setAll(bootSessions);
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
				var bootProjects = boot.projects || [];
				projectStore.setAll(bootProjects);
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

function initSessionTabBar() {
	var bar = S.$("sessionTabBar");
	if (!bar) return;
	var buttons = bar.querySelectorAll(".session-tab");

	function updateActive() {
		var current = sessionStore.sessionListTab.value;
		for (var btn of buttons) {
			btn.classList.toggle("active", btn.dataset.tab === current);
		}
	}

	for (var btn of buttons) {
		btn.addEventListener("click", function () {
			sessionStore.setSessionListTab(this.dataset.tab);
			updateActive();
		});
	}
	updateActive();
}

function startApp() {
	// Mount the reactive SessionList once — signals drive all re-renders.
	var sessionListEl = S.$("sessionList");
	if (sessionListEl) render(html`<${SessionList} />`, sessionListEl);
	initSessionTabBar();

	var path = location.pathname;
	if (path === "/") {
		path = preferredChatPath();
		history.replaceState(null, "", path);
	}
	mount(path);
	connect();
	fetchBootstrap();
	initInstallBanner();
}
