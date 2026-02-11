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
