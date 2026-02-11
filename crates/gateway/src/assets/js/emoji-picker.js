// ── Shared emoji picker component ──────────────────────────
//
// Used by page-settings.js and page-onboarding.js.

import { html } from "htm/preact";
import { useEffect, useRef, useState } from "preact/hooks";

export var EMOJI_LIST = [
	"\u{1f436}",
	"\u{1f431}",
	"\u{1f43b}",
	"\u{1f43a}",
	"\u{1f981}",
	"\u{1f985}",
	"\u{1f989}",
	"\u{1f427}",
	"\u{1f422}",
	"\u{1f40d}",
	"\u{1f409}",
	"\u{1f984}",
	"\u{1f419}",
	"\u{1f41d}",
	"\u{1f98a}",
	"\u{1f43f}\ufe0f",
	"\u{1f994}",
	"\u{1f987}",
	"\u{1f40a}",
	"\u{1f433}",
	"\u{1f42c}",
	"\u{1f99c}",
	"\u{1f9a9}",
	"\u{1f426}",
	"\u{1f40e}",
	"\u{1f98c}",
	"\u{1f418}",
	"\u{1f99b}",
	"\u{1f43c}",
	"\u{1f428}",
	"\u{1f916}",
	"\u{1f47e}",
	"\u{1f47b}",
	"\u{1f383}",
	"\u{2b50}",
	"\u{1f525}",
	"\u{26a1}",
	"\u{1f308}",
	"\u{1f31f}",
	"\u{1f4a1}",
	"\u{1f52e}",
	"\u{1f680}",
	"\u{1f30d}",
	"\u{1f335}",
	"\u{1f33b}",
	"\u{1f340}",
	"\u{1f344}",
	"\u{2744}\ufe0f",
];

export function EmojiPicker({ value, onChange, onSelect }) {
	var [open, setOpen] = useState(false);
	var wrapRef = useRef(null);

	useEffect(() => {
		if (!open) return;
		function onClick(e) {
			if (wrapRef.current && !wrapRef.current.contains(e.target)) setOpen(false);
		}
		document.addEventListener("mousedown", onClick);
		return () => document.removeEventListener("mousedown", onClick);
	}, [open]);

	return html`<div class="settings-emoji-field" ref=${wrapRef}>
		<input
			type="text"
			class="provider-key-input"
			style="width:3.5rem;text-align:center;font-size:1.3rem;padding:0.35rem"
			value=${value || ""}
			onInput=${(e) => onChange(e.target.value)}
			placeholder="\u{1f43e}"
		/>
		<button
			type="button"
			class="provider-btn"
			style="font-size:0.75rem"
			onClick=${() => setOpen(!open)}
		>
			${open ? "Close" : "Pick"}
		</button>
		${
			open
				? html`<div class="settings-emoji-picker">
					${EMOJI_LIST.map(
						(em) =>
							html`<button
								type="button"
								class="settings-emoji-btn ${value === em ? "active" : ""}"
								onClick=${() => {
									onChange(em);
									if (onSelect) onSelect(em);
									setOpen(false);
								}}
							>
							${em}
						</button>`,
					)}
			</div>`
				: null
		}
	</div>`;
}
