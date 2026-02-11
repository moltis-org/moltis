// ── Entry point ────────────────────────────────────────────

import { html } from "htm/preact";
import { render } from "preact";
import prettyBytes from "pretty-bytes";
import { SessionList } from "./components/session-list.js";
import { onEvent } from "./events.js";
import * as gon from "./gon.js";
import { initMobile } from "./mobile.js";
import { updateNavCounts } from "./nav-counts.js";
import { renderSessionProjectSelect } from "./project-combo.js";
import { renderProjectSelect } from "./projects.js";
import { initPWA } from "./pwa.js";
import { initInstallBanner } from "./pwa-install.js";
import { mount, registerPage, sessionPath } from "./router.js";
import { updateSandboxImageUI, updateSandboxUI } from "./sandbox.js";
import { fetchSessions, refreshActiveSession, refreshWelcomeCardIfNeeded, renderSessionList } from "./sessions.js";
import * as S from "./state.js";
import { modelStore } from "./stores/model-store.js";
import { projectStore } from "./stores/project-store.js";
import { sessionStore } from "./stores/session-store.js";
import { initTheme, injectMarkdownStyles } from "./theme.js";
import { connect } from "./websocket.js";

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
initPWA();
initMobile();

// State for favicon/title restoration when switching branches.
var originalFavicons = [];
var originalTitle = document.title;
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
onEvent("session", (payload) => {
	fetchSessions();
	if (payload && payload.kind === "patched" && payload.sessionKey === S.activeSessionKey) {
		refreshActiveSession();
	}
});

function applyMemory(mem) {
	if (!mem) return;
	var el = document.getElementById("memoryInfo");
	if (!el) return;
	var fmt = (b) => prettyBytes(b, { maximumFractionDigits: 0, space: false });
	el.textContent = `${fmt(mem.process)} \u00b7 ${fmt(mem.available)} free / ${fmt(mem.total)}`;
}

applyMemory(gon.get("mem"));
gon.onChange("mem", applyMemory);
onEvent("tick", (payload) => applyMemory(payload.mem));

// Logout button — wire up click handler once.
var logoutBtn = document.getElementById("logoutBtn");
if (logoutBtn) {
	logoutBtn.addEventListener("click", () => {
		fetch("/api/auth/logout", { method: "POST" }).finally(() => {
			location.href = "/";
		});
	});
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
			startApp();
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
		// Show logout button when user authenticated via real credentials
		// (not bypassed via auth_disabled or localhost-no-password).
		if (!auth.auth_disabled && (auth.has_password || auth.has_passkeys) && logoutBtn) {
			logoutBtn.style.display = "";
		}
		if (auth.auth_disabled && !auth.localhost_only) {
			showAuthDisabledBanner();
		}
		startApp();
	})
	.catch(() => {
		// If auth check fails, proceed anyway (backward compat).
		startApp();
	});

function showAuthDisabledBanner() {
	var el = document.getElementById("authDisabledBanner");
	if (el) el.style.display = "";
}

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

function showBranchBanner(branch) {
	var el = document.getElementById("branchBanner");
	if (!el) return;

	// Capture original favicon hrefs on first call
	if (originalFavicons.length === 0) {
		document.querySelectorAll('link[rel="icon"]').forEach((link) => {
			originalFavicons.push({ el: link, href: link.href, type: link.type, sizes: link.sizes?.value });
		});
	}

	if (branch) {
		document.getElementById("branchName").textContent = branch;
		el.style.display = "";

		// Swap favicon to high-contrast branch SVG variant
		document.querySelectorAll('link[rel="icon"]').forEach((link) => {
			link.type = "image/svg+xml";
			link.removeAttribute("sizes");
			link.href = "/assets/icons/icon-branch.svg";
		});

		// Prefix page title with branch name
		var name = document.getElementById("titleName")?.textContent || "moltis";
		document.title = `[${branch}] ${name}`;
	} else {
		el.style.display = "none";

		// Restore original favicons
		originalFavicons.forEach((o) => {
			o.el.type = o.type;
			if (o.sizes) o.el.sizes = o.sizes;
			o.el.href = o.href;
		});

		// Restore original title
		document.title = originalTitle;
	}
}

function applyIdentity(identity) {
	var emojiEl = document.getElementById("titleEmoji");
	var nameEl = document.getElementById("titleName");
	if (emojiEl) emojiEl.textContent = identity?.emoji ? `${identity.emoji} ` : "";
	if (nameEl) nameEl.textContent = identity?.name || "moltis";

	// Keep page title in sync with identity name and branch
	var name = identity?.name || "moltis";
	var branch = gon.get("git_branch");
	if (branch) {
		document.title = `[${branch}] ${name}`;
	} else {
		document.title = name;
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
	fetch("/api/bootstrap")
		.then((r) => r.json())
		.then((boot) => {
			if (boot.channels) S.setCachedChannels(boot.channels.channels || boot.channels || []);
			if (boot.sessions) {
				var bootSessions = boot.sessions || [];
				sessionStore.setAll(bootSessions);
				// Dual-write to state.js for backward compat
				S.setSessions(bootSessions);
				renderSessionList();
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
			/* WS connect will fetch this data anyway */
		});
}

function startApp() {
	// Mount the reactive SessionList once — signals drive all re-renders.
	var sessionListEl = S.$("sessionList");
	if (sessionListEl) render(html`<${SessionList} />`, sessionListEl);

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
