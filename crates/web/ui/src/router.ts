// ── Router ──────────────────────────────────────────────────

import { clearLogsAlert } from "./logs-alert";
import { routes } from "./routes";
import * as S from "./state";

interface PageEntry {
	init: (container: HTMLElement, param?: string | null) => void;
	teardown: () => void;
}

interface PrefixRoute {
	prefix: string;
	init: (container: HTMLElement, param?: string | null) => void;
	teardown: () => void;
}

interface RouteMatch {
	page: PageEntry | null;
	matchedPrefix: string | null;
	param: string | null;
}

const pages: Record<string, PageEntry> = {};
const prefixRoutes: PrefixRoute[] = [];
export let currentPage: string | null = null;
export let currentPrefix: string | null = null;

export function sessionPath(key: string): string {
	return `/chats/${key.replace(/:/g, "/")}`;
}
const pageContent = S.$("pageContent")!;
const sessionsPanel = S.$("sessionsPanel")!;

export function registerPage(
	path: string,
	init: (container: HTMLElement, param?: string | null) => void,
	teardown?: () => void,
): void {
	pages[path] = {
		init: init,
		teardown:
			teardown ||
			(() => {
				/* noop */
			}),
	};
}

export function registerPrefix(
	prefix: string,
	init: (container: HTMLElement, param?: string | null) => void,
	teardown?: () => void,
): void {
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

export function navigate(path: string): void {
	if (path === currentPage) return;
	history.pushState(null, "", path);
	mount(path);
}

function teardownCurrentPage(): void {
	if (!currentPage) return;
	if (pages[currentPage]) {
		pages[currentPage].teardown();
	} else if (currentPrefix) {
		const prevPR = prefixRoutes.find((r) => r.prefix === currentPrefix);
		if (prevPR) prevPR.teardown();
	}
}

function findPageRoute(path: string): RouteMatch {
	const page = pages[path];
	if (page) return { page: page, matchedPrefix: null, param: null };
	for (const pr of prefixRoutes) {
		if (path === pr.prefix || path.indexOf(`${pr.prefix}/`) === 0) {
			const suffix = path.substring(pr.prefix.length + 1);
			const param = suffix ? decodeURIComponent(suffix.replace(/\//g, ":")) : null;
			return { page: pr, matchedPrefix: pr.prefix, param: param };
		}
	}
	return { page: pages["/"] || null, matchedPrefix: null, param: null };
}

function updateNavActiveState(path: string): void {
	const links = document.querySelectorAll(".nav-link");
	links.forEach((a) => {
		const href = a.getAttribute("href");
		const active = href === path || (href !== "/" && href != null && path.indexOf(href) === 0);
		a.classList.toggle("active", active);
	});

	const settingsBtn = document.getElementById("settingsBtn");
	if (settingsBtn) {
		const settingsActive =
			path === routes.settings || (routes.settings != null && path.indexOf(`${routes.settings}/`) === 0);
		settingsBtn.classList.toggle("active", settingsActive);
	}
}

export function mount(path: string): void {
	const route = findPageRoute(path);
	const page = route.page;
	const samePrefixNav = route.matchedPrefix && route.matchedPrefix === currentPrefix;

	if (!samePrefixNav) {
		teardownCurrentPage();
		pageContent.textContent = "";
		pageContent.style.cssText = "";
	}

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
const burgerBtn = S.$("burgerBtn");
const navPanel = S.$("navPanel");

if (burgerBtn && navPanel) {
	burgerBtn.addEventListener("click", () => {
		navPanel.classList.toggle("hidden");
	});
}

if (navPanel) {
	navPanel.addEventListener("click", (e: Event) => {
		const link = (e.target as HTMLElement).closest("[data-nav]");
		if (!link) return;
		e.preventDefault();
		const href = link.getAttribute("href");
		if (href) navigate(href);
	});
}

const titleLink = document.getElementById("titleLink");
if (titleLink) {
	titleLink.addEventListener("click", (e: Event) => {
		e.preventDefault();
		navigate(routes.chats || "/chats");
	});
}
