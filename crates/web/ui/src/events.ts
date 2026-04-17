// ── Event bus (pub/sub for WebSocket events) ─────────────────

export type EventHandler = (payload: unknown) => void;

export const eventListeners: Record<string, EventHandler[]> = {};

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
