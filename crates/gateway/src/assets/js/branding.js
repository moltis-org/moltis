function trimString(value) {
	return typeof value === "string" ? value.trim() : "";
}

export function identityName(identity) {
	var name = trimString(identity?.name);
	return name || "moltis";
}

export function identityEmoji(identity) {
	return trimString(identity?.emoji);
}

export function identityUserName(identity) {
	return trimString(identity?.user_name);
}

export function formatPageTitle(identity) {
	return identityName(identity);
}

export function formatLoginTitle(identity) {
	return identityName(identity);
}

function escapeSvgText(text) {
	return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

export function applyIdentityFavicon(identity) {
	var emoji = identityEmoji(identity);
	if (!emoji) return false;

	var links = Array.from(document.querySelectorAll('link[rel="icon"]'));
	if (links.length === 0) {
		var fallback = document.createElement("link");
		fallback.rel = "icon";
		document.head.appendChild(fallback);
		links = [fallback];
	}

	var safeEmoji = escapeSvgText(emoji);
	var svg =
		`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">` +
		`<text x="50%" y="50%" text-anchor="middle" dominant-baseline="central" font-size="52">${safeEmoji}</text>` +
		`</svg>`;
	var href = `data:image/svg+xml,${encodeURIComponent(svg)}`;
	var forceResetHref = "data:image/gif;base64,R0lGODlhAQABAAAAACwAAAAAAQABAAA=";

	for (var link of links) {
		link.type = "image/svg+xml";
		link.removeAttribute("sizes");
		link.href = forceResetHref;
		link.href = href;
	}
	return true;
}
