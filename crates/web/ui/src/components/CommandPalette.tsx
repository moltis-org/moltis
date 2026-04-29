// ── Command Palette ──────────────────────────────────────────
//
// Cmd+K / Ctrl+K overlay for quick navigation, actions, and
// session search. Rendered as a signal-driven Preact component
// inside GlobalDialogs.

import type { VNode } from "preact";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import { sendRpc } from "../helpers";
import { t } from "../i18n";
import { navigate, sessionPath } from "../router";
import { buildCommands, type Command, closePalette, paletteOpen } from "../stores/command-store";
import { sessionStore } from "../stores/session-store";

// ── Types ───────────────────────────────────────��────────────

interface SessionHit {
	label?: string;
	sessionKey: string;
	snippet: string;
}

type PaletteItem = { type: "command"; cmd: Command } | { type: "session"; hit: SessionHit };

// ── Constants ────────────────────────────────────────────────

const GROUP_LABELS: Record<string, string> = {
	navigation: "Navigation",
	settings: "Settings",
	actions: "Actions",
	sessions: "Sessions",
};

const GROUP_ORDER = ["navigation", "settings", "actions", "sessions"];

// ── Item renderer ────────────────────────────────────────────

interface PaletteItemRowProps {
	item: PaletteItem;
	active: boolean;
	onSelect: () => void;
	onHover: () => void;
}

function PaletteItemRow({ item, active, onSelect, onHover }: PaletteItemRowProps): VNode {
	const cls = `cmd-palette-item ${active ? "cmd-palette-item-active" : ""}`;
	if (item.type === "command") {
		return (
			<div
				role="option"
				tabIndex={-1}
				aria-selected={active}
				class={cls}
				data-active={active ? "true" : undefined}
				onClick={onSelect}
				onKeyDown={(e: KeyboardEvent) => {
					if (e.key === "Enter") onSelect();
				}}
				onMouseEnter={onHover}
			>
				{item.cmd.icon && <span class={`icon icon-sm ${item.cmd.icon}`} />}
				<span class="cmd-palette-item-label">{item.cmd.label}</span>
				{item.cmd.shortcut && <kbd class="cmd-palette-kbd">{item.cmd.shortcut}</kbd>}
			</div>
		);
	}
	const hit = item.hit;
	return (
		<div
			role="option"
			tabIndex={-1}
			aria-selected={active}
			class={cls}
			data-active={active ? "true" : undefined}
			onClick={onSelect}
			onKeyDown={(e: KeyboardEvent) => {
				if (e.key === "Enter") onSelect();
			}}
			onMouseEnter={onHover}
		>
			<span class="icon icon-sm icon-chat" />
			<span class="cmd-palette-item-label">{hit.label || hit.sessionKey}</span>
			<span class="cmd-palette-item-hint">{truncate(hit.snippet, 60)}</span>
		</div>
	);
}

// ── Main component ───────────────────────────────────────────

export function CommandPalette(): VNode | null {
	const show = paletteOpen.value;
	const [query, setQuery] = useState("");
	const [activeIdx, setActiveIdx] = useState(0);
	const [sessionHits, setSessionHits] = useState<SessionHit[]>([]);
	const inputRef = useRef<HTMLInputElement>(null);
	const listRef = useRef<HTMLDivElement>(null);
	const searchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
	const reqIdRef = useRef(0);

	const commands = useMemo(() => buildCommands(), [show]);

	const filtered = useMemo(() => {
		if (!query) return commands;
		const q = query.toLowerCase();
		return commands.filter((cmd) => {
			if (cmd.label.toLowerCase().includes(q)) return true;
			if (cmd.group.toLowerCase().includes(q)) return true;
			return cmd.keywords?.some((kw) => kw.includes(q)) ?? false;
		});
	}, [commands, query]);

	const allItems = useMemo<PaletteItem[]>(() => {
		const items: PaletteItem[] = filtered.map((cmd) => ({ type: "command" as const, cmd }));
		for (const hit of sessionHits) {
			items.push({ type: "session", hit });
		}
		return items;
	}, [filtered, sessionHits]);

	// Build a flat ordered list following GROUP_ORDER so render index
	// always matches the position used by execute()/setActiveIdx().
	const orderedItems = useMemo(() => {
		const groups: Record<string, PaletteItem[]> = {};
		for (const item of allItems) {
			const group = item.type === "command" ? item.cmd.group : "sessions";
			if (!groups[group]) groups[group] = [];
			groups[group].push(item);
		}
		const ordered: Array<{ group: string; items: PaletteItem[] }> = [];
		for (const group of GROUP_ORDER) {
			if (groups[group]?.length) {
				ordered.push({ group, items: groups[group] });
			}
		}
		return ordered;
	}, [allItems]);

	useLayoutEffect(() => {
		if (show) {
			setQuery("");
			setActiveIdx(0);
			setSessionHits([]);
			inputRef.current?.focus();
		}
	}, [show]);

	useEffect(() => {
		setActiveIdx(0);
	}, [query, sessionHits.length]);

	useEffect(() => {
		if (searchTimer.current) clearTimeout(searchTimer.current);
		if (query.length < 2) {
			setSessionHits([]);
			return;
		}
		searchTimer.current = setTimeout(() => {
			const thisReq = ++reqIdRef.current;
			sendRpc<SessionHit[]>("sessions.search", {
				query,
				includeArchived: sessionStore.showArchivedSessions.value,
			})
				.then((res) => {
					if (thisReq !== reqIdRef.current) return;
					if (res?.ok && Array.isArray(res.payload)) {
						setSessionHits(res.payload.slice(0, 5));
					} else {
						setSessionHits([]);
					}
				})
				.catch(() => setSessionHits([]));
		}, 300);
		return () => {
			if (searchTimer.current) clearTimeout(searchTimer.current);
		};
	}, [query]);

	useEffect(() => {
		if (!listRef.current) return;
		const active = listRef.current.querySelector("[data-active='true']");
		if (active) active.scrollIntoView({ block: "nearest" });
	}, [activeIdx]);

	// Flat list in render order for index-based execution.
	const flatItems = useMemo(() => orderedItems.flatMap((g) => g.items), [orderedItems]);

	// Refs keep the capture-phase document listener always up-to-date
	// without re-registering on every render.
	const flatItemsRef = useRef<PaletteItem[]>([]);
	flatItemsRef.current = flatItems;
	const activeIdxRef = useRef(0);
	activeIdxRef.current = activeIdx;

	function execute(idx: number): void {
		const item = flatItemsRef.current[idx];
		if (!item) return;
		closePalette();
		if (item.type === "command") {
			item.cmd.action();
		} else {
			navigate(sessionPath(item.hit.sessionKey));
		}
	}

	// Capture-phase document listener intercepts navigation keys before
	// the browser's native input handling (headless Chromium's autocomplete
	// consumes ArrowDown/Up on <input> elements even with autocomplete="off").
	useEffect(() => {
		if (!show) return;

		function handleKeyDown(e: KeyboardEvent) {
			if (e.key === "Escape") {
				e.preventDefault();
				closePalette();
			} else if (e.key === "ArrowDown") {
				e.preventDefault();
				setActiveIdx((i) => Math.min(i + 1, flatItemsRef.current.length - 1));
			} else if (e.key === "ArrowUp") {
				e.preventDefault();
				setActiveIdx((i) => Math.max(i - 1, 0));
			} else if (e.key === "Enter") {
				e.preventDefault();
				execute(activeIdxRef.current);
			}
		}

		document.addEventListener("keydown", handleKeyDown, true);
		return () => document.removeEventListener("keydown", handleKeyDown, true);
	}, [show]);

	if (!show) return null;

	// Pre-compute flat index offset for each group so render stays
	// in sync with flatItems without an imperative counter.
	let groupOffset = 0;

	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: backdrop dismiss pattern, keyboard handled by inner dialog
		// biome-ignore lint/a11y/useKeyWithClickEvents: keyboard handled by capture-phase document listener
		<div
			class="cmd-palette-backdrop"
			onClick={(e: Event) => {
				if (e.target === e.currentTarget) closePalette();
			}}
		>
			<div role="dialog" aria-modal="true" aria-label="Command palette" class="cmd-palette">
				<div class="cmd-palette-input-row">
					<span class="icon icon-md icon-search cmd-palette-search-icon" />
					<input
						ref={inputRef}
						class="cmd-palette-input"
						type="text"
						autocomplete="off"
						placeholder={t("common:actions.search") || "Search\u2026"}
						value={query}
						onInput={(e: Event) => setQuery((e.target as HTMLInputElement).value)}
					/>
					<kbd class="cmd-palette-kbd">esc</kbd>
				</div>
				<div class="cmd-palette-list" ref={listRef} role="listbox">
					{flatItems.length === 0 && <div class="cmd-palette-empty">{t("common:labels.noMatches")}</div>}
					{orderedItems.map(({ group, items }) => {
						const baseIdx = groupOffset;
						groupOffset += items.length;
						return (
							<fieldset key={group} aria-label={GROUP_LABELS[group]} class="cmd-palette-fieldset">
								<div class="cmd-palette-group">{GROUP_LABELS[group]}</div>
								{items.map((item, i) => {
									const idx = baseIdx + i;
									const key = item.type === "command" ? item.cmd.id : item.hit.sessionKey;
									return (
										<PaletteItemRow
											key={key}
											item={item}
											active={idx === activeIdx}
											onSelect={() => execute(idx)}
											onHover={() => setActiveIdx(idx)}
										/>
									);
								})}
							</fieldset>
						);
					})}
				</div>
			</div>
		</div>
	);
}

function truncate(text: string, max: number): string {
	if (text.length <= max) return text;
	return `${text.slice(0, max)}\u2026`;
}
