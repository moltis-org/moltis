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

function emojiFaviconPng(emoji) {
	var canvas = document.createElement("canvas");
	canvas.width = 64;
	canvas.height = 64;
	var ctx = canvas.getContext("2d");
	if (!ctx) return null;
	ctx.clearRect(0, 0, 64, 64);
	ctx.textAlign = "center";
	ctx.textBaseline = "middle";
	ctx.font = "52px 'Apple Color Emoji','Segoe UI Emoji','Noto Color Emoji',sans-serif";
	ctx.fillText(emoji, 32, 34);
	return canvas.toDataURL("image/png");
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

	var href = emojiFaviconPng(emoji);
	if (!href) return false;

	for (var link of links) {
		link.type = "image/png";
		link.removeAttribute("sizes");
		link.href = href;
	}
	return true;
}
