import type { ResolvedIdentity } from "./types/gon";

function trimString(value: unknown): string {
	return typeof value === "string" ? value.trim() : "";
}

export function identityName(identity: Partial<ResolvedIdentity> | null | undefined): string {
	const name = trimString(identity?.name);
	return name || "moltis";
}

export function identityEmoji(identity: Partial<ResolvedIdentity> | null | undefined): string {
	return trimString(identity?.emoji);
}

export function identityUserName(identity: Partial<ResolvedIdentity> | null | undefined): string {
	return trimString(identity?.user_name);
}

export function formatPageTitle(identity: Partial<ResolvedIdentity> | null | undefined): string {
	return identityName(identity);
}

export function formatLoginTitle(identity: Partial<ResolvedIdentity> | null | undefined): string {
	return identityName(identity);
}

function emojiFaviconPng(emoji: string): string | null {
	const canvas = document.createElement("canvas");
	canvas.width = 64;
	canvas.height = 64;
	const ctx = canvas.getContext("2d");
	if (!ctx) return null;
	ctx.clearRect(0, 0, 64, 64);
	ctx.textAlign = "center";
	ctx.textBaseline = "middle";
	ctx.font = "52px 'Apple Color Emoji','Segoe UI Emoji','Noto Color Emoji',sans-serif";
	ctx.fillText(emoji, 32, 34);
	return canvas.toDataURL("image/png");
}

export function applyIdentityFavicon(identity: Partial<ResolvedIdentity> | null | undefined): boolean {
	const emoji = identityEmoji(identity);
	if (!emoji) return false;

	let links = Array.from(document.querySelectorAll<HTMLLinkElement>('link[rel="icon"]'));
	if (links.length === 0) {
		const fallback = document.createElement("link");
		fallback.rel = "icon";
		document.head.appendChild(fallback);
		links = [fallback];
	}

	const href = emojiFaviconPng(emoji);
	if (!href) return false;

	for (const link of links) {
		link.type = "image/png";
		link.removeAttribute("sizes");
		link.href = href;
	}
	return true;
}
