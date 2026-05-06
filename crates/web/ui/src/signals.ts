// ── Preact signal bridge for shared state ─────────────────────
// Mirrors key state.js vars as Preact signals so that both imperative
// code (websocket.js) and Preact pages can coexist during migration.
//
// Signals for models, projects, sessions, selectedModelId, and
// activeSessionKey have moved to stores/*.js. They are re-exported
// here for backward compat with pages that still import from signals.js.

import type { Signal } from "@preact/signals";
import { signal } from "@preact/signals";
import { models, selectedModelId } from "./stores/model-store";
import { projects } from "./stores/project-store";
import { activeSessionKey, sessions } from "./stores/session-store";

export { activeSessionKey, models, projects, selectedModelId, sessions };

// Signals that haven't moved to stores yet
export const connected: Signal<boolean> = signal(false);
export const cachedChannels: Signal<unknown | null> = signal(null);
export const unseenErrors: Signal<number> = signal(0);
export const unseenWarns: Signal<number> = signal(0);
export const sandboxInfo: Signal<unknown | null> = signal(null);
