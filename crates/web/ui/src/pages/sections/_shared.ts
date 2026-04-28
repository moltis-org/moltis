// ── Shared state and helpers for settings sections ───────────
//
// This module holds module-level signals, types, and tiny helpers that
// multiple section components reference.  By extracting them here we
// avoid circular imports between SettingsPage.tsx and the individual
// section files.

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { sendRpc } from "../../helpers";
import * as S from "../../state";

// ── Types ────────────────────────────────────────────────────

export interface IdentityData {
	name?: string;
	emoji?: string;
	theme?: string;
	user_name?: string;
	soul?: string;
	[key: string]: unknown;
}

import type { RpcResponse } from "../../types/rpc";
export type { RpcResponse };

export interface SectionItem {
	id?: string;
	label?: string;
	icon?: VNode;
	page?: boolean;
	group?: string;
}

// ── Module-level signals ─────────────────────────────────────

export const identity = signal<IdentityData | null>(null);
export const loading = signal(true);
export const activeSection = signal("profile");
export const activeSubPath = signal("");
export const mobileSidebarVisible = signal(true);

// ── Mount state ──────────────────────────────────────────────

let _mounted = false;
let _containerRef: HTMLElement | null = null;

export function isMounted(): boolean {
	return _mounted;
}

export function setMounted(v: boolean): void {
	_mounted = v;
}

export function getContainerRef(): HTMLElement | null {
	return _containerRef;
}

export function setContainerRef(el: HTMLElement | null): void {
	_containerRef = el;
}

// ── Render helper ────────────────────────────────────────────
// `rerender` is supplied by SettingsPage after it mounts via
// `setRerenderFn`.  This avoids the circular import that would
// occur if sections imported the <SettingsPage/> component.

let _rerenderFn: (() => void) | null = null;

export function setRerenderFn(fn: () => void): void {
	_rerenderFn = fn;
}

export function rerender(): void {
	if (_rerenderFn) _rerenderFn();
}

// ── Utility helpers ──────────────────────────────────────────

export function isMobileViewport(): boolean {
	return window.innerWidth < 768;
}

export function isSafariBrowser(): boolean {
	if (typeof navigator === "undefined") return false;
	const ua = navigator.userAgent || "";
	const vendor = navigator.vendor || "";
	if (!ua.includes("Safari/")) return false;
	if (/(Chrome|CriOS|Chromium|Edg|OPR|FxiOS|Firefox|SamsungBrowser)/.test(ua)) return false;
	return /Apple/i.test(vendor) || ua.includes("Safari/");
}

export function isMissingMethodError(res: RpcResponse | null): boolean {
	const message = res?.error?.message;
	if (typeof message !== "string") return false;
	const lower = message.toLowerCase();
	return lower.includes("method") && (lower.includes("not found") || lower.includes("unknown"));
}

export function fetchMainIdentity(): Promise<RpcResponse> {
	return sendRpc("agents.identity.get", { agent_id: "main" }).then((res: RpcResponse) => {
		if (res?.ok || !isMissingMethodError(res)) return res;
		return sendRpc("agent.identity.get", {});
	});
}

export function fetchIdentity(): void {
	if (!_mounted) return;
	fetchMainIdentity().then((res: RpcResponse) => {
		if (res?.ok) {
			identity.value = res.payload as IdentityData;
			loading.value = false;
			rerender();
		} else if (_mounted && !S.connected) {
			setTimeout(fetchIdentity, 500);
		} else {
			loading.value = false;
			rerender();
		}
	});
}
