// ── Session search ──────────────────────────────────────────

import { esc, sendRpc } from "./helpers";
import { currentPrefix, navigate, sessionPath } from "./router";
import { switchSession } from "./sessions";
import * as S from "./state";
import { sessionStore } from "./stores/session-store";

interface SearchHit {
	label?: string;
	sessionKey: string;
	snippet: string;
	role: string;
	messageIndex: number;
}

interface SearchContext {
	query: string;
	messageIndex: number;
}

const searchInput = S.$<HTMLInputElement>("sessionSearch")!;
const searchResults = S.$("searchResults")!;
searchResults.className = "search-dropdown hidden";
let searchTimer: ReturnType<typeof setTimeout> | null = null;
let searchHits: SearchHit[] = [];
let searchIdx = -1;

function debounceSearch(): void {
	if (searchTimer !== null) clearTimeout(searchTimer);
	searchTimer = setTimeout(doSearch, 300);
}

function doSearch(): void {
	const q = searchInput.value.trim();
	if (!(q && S.connected)) {
		hideSearch();
		return;
	}
	sendRpc("sessions.search", {
		query: q,
		includeArchived: sessionStore.showArchivedSessions.value,
	}).then((res) => {
		if (!res?.ok) {
			hideSearch();
			return;
		}
		searchHits = (res.payload as SearchHit[] | undefined) || [];
		searchIdx = -1;
		renderSearchResults(q);
	});
}

function hideSearch(): void {
	searchResults.classList.add("hidden");
	searchHits = [];
	searchIdx = -1;
}

function renderSearchResults(query: string): void {
	searchResults.textContent = "";
	if (searchHits.length === 0) {
		const empty = document.createElement("div");
		empty.className = "search-hit-empty";
		empty.textContent = "No results";
		searchResults.appendChild(empty);
		searchResults.classList.remove("hidden");
		return;
	}
	searchHits.forEach((hit, i) => {
		const el = document.createElement("div");
		el.className = "search-hit";
		el.setAttribute("data-idx", String(i));

		const lbl = document.createElement("div");
		lbl.className = "search-hit-label";
		lbl.textContent = hit.label || hit.sessionKey;
		el.appendChild(lbl);

		// Safe: esc() escapes all HTML entities first, then we only wrap
		// the already-escaped query substring in <mark> tags.
		const snip = document.createElement("div");
		snip.className = "search-hit-snippet";
		const escaped = esc(hit.snippet);
		const qEsc = esc(query);
		const re = new RegExp(`(${qEsc.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "gi");
		// Safe: both `escaped` and `qEsc` are HTML-entity-escaped by esc(),
		// so <mark> wrapping cannot introduce script injection.
		// This is a mechanical TypeScript conversion of existing safe code.
		const highlighted = escaped.replace(re, "<mark>$1</mark>");
		snip.insertAdjacentHTML("beforeend", highlighted);
		el.appendChild(snip);

		const role = document.createElement("div");
		role.className = "search-hit-role";
		role.textContent = hit.role;
		el.appendChild(role);

		el.addEventListener("click", () => {
			const ctx: SearchContext = { query, messageIndex: hit.messageIndex };
			if (currentPrefix !== "/chats") {
				sessionStorage.setItem("moltis-search-ctx", JSON.stringify(ctx));
				navigate(sessionPath(hit.sessionKey));
			} else {
				switchSession(hit.sessionKey, ctx);
			}
			searchInput.value = "";
			hideSearch();
		});

		searchResults.appendChild(el);
	});
	searchResults.classList.remove("hidden");
}

function updateSearchActive(): void {
	const items = searchResults.querySelectorAll<HTMLElement>(".search-hit");
	items.forEach((el, i) => {
		el.classList.toggle("active", i === searchIdx);
	});
	if (searchIdx >= 0 && items[searchIdx]) {
		items[searchIdx].scrollIntoView({ block: "nearest" });
	}
}

searchInput.addEventListener("input", debounceSearch);
searchInput.addEventListener("keydown", (e: KeyboardEvent) => {
	if (searchResults.classList.contains("hidden")) return;
	if (e.key === "ArrowDown") {
		e.preventDefault();
		searchIdx = Math.min(searchIdx + 1, searchHits.length - 1);
		updateSearchActive();
	} else if (e.key === "ArrowUp") {
		e.preventDefault();
		searchIdx = Math.max(searchIdx - 1, 0);
		updateSearchActive();
	} else if (e.key === "Enter") {
		e.preventDefault();
		if (searchIdx >= 0 && searchHits[searchIdx]) {
			const h = searchHits[searchIdx];
			const ctx: SearchContext = {
				query: searchInput.value.trim(),
				messageIndex: h.messageIndex,
			};
			if (currentPrefix !== "/chats") {
				sessionStorage.setItem("moltis-search-ctx", JSON.stringify(ctx));
				navigate(sessionPath(h.sessionKey));
			} else {
				switchSession(h.sessionKey, ctx);
			}
			searchInput.value = "";
			hideSearch();
		}
	} else if (e.key === "Escape") {
		searchInput.value = "";
		hideSearch();
	}
});

document.addEventListener("click", (e: MouseEvent) => {
	if (!(searchInput.contains(e.target as Node) || searchResults.contains(e.target as Node))) {
		hideSearch();
	}
});
