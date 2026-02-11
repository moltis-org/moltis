// ── Router ──────────────────────────────────────────────────

import { clearLogsAlert } from "./logs-alert.js";
import { routes } from "./routes.js";
import * as S from "./state.js";

var pages = {};
var prefixRoutes = [];
export var currentPage = null;
export var currentPrefix = null;

export function sessionPath(key) {
	return `/chats/${key.replace(/:/g, "/")}`;
}
var pageContent = S.$("pageContent");
var sessionsPanel = S.$("sessionsPanel");

export function registerPage(path, init, teardown) {
	pages[path] = {
		init: init,
		teardown:
			teardown ||
			(() => {
				/* noop */
			}),
	};
}

export function registerPrefix(prefix, init, teardown) {
	prefixRoutes.push({
		prefix: prefix,
		init: init,
		teardown:
			teardown ||
			(() => {
				/* noop */
			}),
	});
}

export function navigate(path) {
	if (path === currentPage) return;
	history.pushState(null, "", path);
	mount(path);
}

function teardownCurrentPage() {
	if (!currentPage) return;
	if (pages[currentPage]) {
		pages[currentPage].teardown();
	} else if (currentPrefix) {
		var prevPR = prefixRoutes.find((r) => r.prefix === currentPrefix);
		if (prevPR) prevPR.teardown();
	}
}

function findPageRoute(path) {
	var page = pages[path];
	if (page) return { page: page, matchedPrefix: null, param: null };
	for (var pr of prefixRoutes) {
		if (path === pr.prefix || path.indexOf(`${pr.prefix}/`) === 0) {
			var suffix = path.substring(pr.prefix.length + 1);
			var param = suffix ? decodeURIComponent(suffix.replace(/\//g, ":")) : null;
			return { page: pr, matchedPrefix: pr.prefix, param: param };
		}
	}
	return { page: pages["/"] || null, matchedPrefix: null, param: null };
}

function updateNavActiveState(path) {
	var links = document.querySelectorAll(".nav-link");
	links.forEach((a) => {
		var href = a.getAttribute("href");
		var active = href === path || (href !== "/" && path.indexOf(href) === 0);
		a.classList.toggle("active", active);
	});

	var settingsBtn = document.getElementById("settingsBtn");
	if (settingsBtn) {
		var settingsActive = path === routes.settings || path.indexOf(`${routes.settings}/`) === 0;
		settingsBtn.classList.toggle("active", settingsActive);
	}
}

export function mount(path) {
	teardownCurrentPage();
	pageContent.textContent = "";
	pageContent.style.cssText = "";
	currentPrefix = null;

	var route = findPageRoute(path);
	var page = route.page;

	currentPage = path;
	currentPrefix = route.matchedPrefix;

	updateNavActiveState(path);

	// Show sessions panel on chat pages
	if (route.matchedPrefix === routes.chats || path === routes.chats) {
		sessionsPanel.classList.remove("hidden");
	} else {
		sessionsPanel.classList.add("hidden");
	}

	// Clear unseen logs alert when viewing the logs page
	if (path === "/logs" || path === routes.logs) clearLogsAlert();

	if (page) page.init(pageContent, route.param);
}

window.addEventListener("popstate", () => {
	mount(location.pathname);
});

// ── Nav panel (burger toggle) ────────────────────────────────
var burgerBtn = S.$("burgerBtn");
var navPanel = S.$("navPanel");

if (burgerBtn && navPanel) {
	burgerBtn.addEventListener("click", () => {
		navPanel.classList.toggle("hidden");
	});
}

if (navPanel) {
	navPanel.addEventListener("click", (e) => {
		var link = e.target.closest("[data-nav]");
		if (!link) return;
		e.preventDefault();
		navigate(link.getAttribute("href"));
	});
}

var titleLink = document.getElementById("titleLink");
if (titleLink) {
	titleLink.addEventListener("click", (e) => {
		e.preventDefault();
		navigate(routes.chats);
	});
}
