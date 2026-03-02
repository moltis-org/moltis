// ── Channel store (signal-based) ─────────────────────────────
//
// Single source of truth for channel data.
// Centralizes signals previously local to page-channels.js.

import { signal } from "@preact/signals";

// ── Signals ──────────────────────────────────────────────────
export var channels = signal([]);
export var senders = signal([]);
export var activeTab = signal("channels");
export var cachedChannels = signal(null);

// ── Methods ──────────────────────────────────────────────────

export function setChannels(arr) {
	channels.value = arr || [];
}

export function setSenders(arr) {
	senders.value = arr || [];
}

export function setActiveTab(tab) {
	activeTab.value = tab || "channels";
}

export function setCachedChannels(v) {
	cachedChannels.value = v;
}

export var channelStore = {
	channels,
	senders,
	activeTab,
	cachedChannels,
	setChannels,
	setSenders,
	setActiveTab,
	setCachedChannels,
};
