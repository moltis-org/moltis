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
	var name = identityName(identity);
	var userName = identityUserName(identity);
	if (userName) return `${name}: ${userName} AI assistant`;
	return `${name}: AI assistant`;
}

export function formatLoginTitle(identity) {
	return identityName(identity);
}

function escapeSvgText(text) {
	return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function emojiFaviconHref(emoji) {
	var safeEmoji = escapeSvgText(emoji);
	var svg =
		`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">` +
		`<text x="50%" y="50%" text-anchor="middle" dominant-baseline="central" font-size="52">${safeEmoji}</text>` +
		`</svg>`;
	return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export function applyIdentityFavicon(identity) {
	var emoji = identityEmoji(identity);
	if (!emoji) return false;

	var links = Array.from(document.querySelectorAll('link[rel="icon"]'));
	if (links.length === 0) {
		var fallbackLink = document.createElement("link");
		fallbackLink.rel = "icon";
		document.head.appendChild(fallbackLink);
		links = [fallbackLink];
	}

	var href = emojiFaviconHref(emoji);
	for (var link of links) {
		link.type = "image/svg+xml";
		link.removeAttribute("sizes");
		link.href = href;
	}
	return true;
}
