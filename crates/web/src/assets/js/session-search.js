// ── Session search ──────────────────────────────────────────

import { esc, sendRpc } from "./helpers.js";
import { currentPrefix, navigate, sessionPath } from "./router.js";
import { switchSession } from "./sessions.js";
import * as S from "./state.js";

var searchInput = S.$("sessionSearch");
var searchResults = S.$("searchResults");
searchResults.className = "search-dropdown hidden";
var searchTimer = null;
var searchHits = [];
var searchIdx = -1;

function debounceSearch() {
	clearTimeout(searchTimer);
	searchTimer = setTimeout(doSearch, 300);
}

function doSearch() {
	var q = searchInput.value.trim();
	if (!(q && S.connected)) {
		hideSearch();
		return;
	}
	sendRpc("sessions.search", { query: q }).then((res) => {
		if (!res?.ok) {
			hideSearch();
			return;
		}
		searchHits = res.payload || [];
		searchIdx = -1;
		renderSearchResults(q);
	});
}

function hideSearch() {
	searchResults.classList.add("hidden");
	searchHits = [];
	searchIdx = -1;
}

function renderSearchResults(query) {
	searchResults.textContent = "";
	if (searchHits.length === 0) {
		var empty = document.createElement("div");
		empty.className = "search-hit-empty";
		empty.textContent = "No results";
		searchResults.appendChild(empty);
		searchResults.classList.remove("hidden");
		return;
	}
	searchHits.forEach((hit, i) => {
		var el = document.createElement("div");
		el.className = "search-hit";
		el.setAttribute("data-idx", i);

		var lbl = document.createElement("div");
		lbl.className = "search-hit-label";
		lbl.textContent = hit.label || hit.sessionKey;
		el.appendChild(lbl);

		// Safe: esc() escapes all HTML entities first, then we only wrap
		// the already-escaped query substring in <mark> tags.
		var snip = document.createElement("div");
		snip.className = "search-hit-snippet";
		var escaped = esc(hit.snippet);
		var qEsc = esc(query);
		var re = new RegExp(`(${qEsc.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "gi");
		// Safe: both `escaped` and `qEsc` are HTML-entity-escaped by esc(),
		// so <mark> wrapping cannot introduce script injection.
		snip.innerHTML = escaped.replace(re, "<mark>$1</mark>");
		el.appendChild(snip);

		var role = document.createElement("div");
		role.className = "search-hit-role";
		role.textContent = hit.role;
		el.appendChild(role);

		el.addEventListener("click", () => {
			var ctx = { query: query, messageIndex: hit.messageIndex };
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

function updateSearchActive() {
	var items = searchResults.querySelectorAll(".search-hit");
	items.forEach((el, i) => {
		el.classList.toggle("active", i === searchIdx);
	});
	if (searchIdx >= 0 && items[searchIdx]) {
		items[searchIdx].scrollIntoView({ block: "nearest" });
	}
}

searchInput.addEventListener("input", debounceSearch);
searchInput.addEventListener("keydown", (e) => {
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
			var h = searchHits[searchIdx];
			var ctx = {
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

document.addEventListener("click", (e) => {
	if (!(searchInput.contains(e.target) || searchResults.contains(e.target))) {
		hideSearch();
	}
});
