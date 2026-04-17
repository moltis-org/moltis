// ── Channel store (signal-based) ─────────────────────────────
//
// Single source of truth for channel data.
// Centralizes signals previously local to page-channels.js.

import { signal } from "@preact/signals";
import type { ChannelInfo, SenderInfo } from "../types";

// ── Signals ──────────────────────────────────────────────────
export const channels = signal<ChannelInfo[]>([]);
export const senders = signal<SenderInfo[]>([]);
export const activeTab = signal<string>("channels");
export const cachedChannels = signal<ChannelInfo[] | null>(null);

// ── Methods ──────────────────────────────────────────────────

export function setChannels(arr: ChannelInfo[]): void {
	channels.value = arr || [];
}

export function setSenders(arr: SenderInfo[]): void {
	senders.value = arr || [];
}

export function setActiveTab(tab: string): void {
	activeTab.value = tab || "channels";
}

export function setCachedChannels(v: ChannelInfo[] | null): void {
	cachedChannels.value = v;
}

export const channelStore = {
	channels,
	senders,
	activeTab,
	cachedChannels,
	setChannels,
	setSenders,
	setActiveTab,
	setCachedChannels,
};
