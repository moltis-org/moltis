// ── Event bus (pub/sub for WebSocket events) ─────────────────

import type { WsEventName, WsEventPayloadMap } from "./types/ws-events";

export type EventHandler = (payload: unknown) => void;

export const eventListeners: Record<string, EventHandler[]> = {};

/**
 * Subscribe to a WebSocket event. When called with a `WsEventName` enum value
 * the handler receives the correct payload type. Plain strings are still
 * accepted for events not (yet) in the enum.
 */
export function onEvent<E extends WsEventName>(
	eventName: E,
	handler: (payload: WsEventPayloadMap[E]) => void,
): () => void;
export function onEvent(eventName: string, handler: EventHandler): () => void;
export function onEvent(eventName: string, handler: EventHandler): () => void {
	(eventListeners[eventName] = eventListeners[eventName] || []).push(handler);
	return function off(): void {
		const arr = eventListeners[eventName];
		if (arr) {
			const idx = arr.indexOf(handler);
			if (idx !== -1) arr.splice(idx, 1);
		}
	};
}
