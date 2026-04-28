// ── Syntax highlighting (Shiki) ────────────────────────────
//
// Lazy-loads the Shiki highlighter on app startup. Code blocks
// rendered during streaming show a language badge but no colors;
// highlighting is applied after the stream completes or when
// history messages are loaded.

import type { BundledLanguage, BundledTheme, HighlighterGeneric } from "shiki";

let highlighter: HighlighterGeneric<BundledLanguage, BundledTheme> | null = null;
let highlighterInitPromise: Promise<void> | null = null;
const languageLoadPromises: Map<string, Promise<void>> = new Map();

/**
 * Initialize the Shiki highlighter. Call once at app startup (fire-and-forget).
 * Safe to call multiple times -- subsequent calls are no-ops.
 */
export async function initHighlighter(): Promise<HighlighterGeneric<BundledLanguage, BundledTheme> | null> {
	if (highlighter) return highlighter;
	if (highlighterInitPromise) {
		await highlighterInitPromise;
		return highlighter;
	}
	highlighterInitPromise = (async (): Promise<void> => {
		try {
			const shiki = await import("shiki");
			// Load only themes at startup; grammars are loaded on demand per language.
			highlighter = await shiki.createHighlighter({
				themes: ["github-dark", "github-light"],
				langs: [],
			});
		} catch (err) {
			console.warn("[shiki] failed to initialize highlighter:", err);
		}
	})();
	await highlighterInitPromise;
	return highlighter;
}

/** Returns whether the highlighter has finished loading. */
export function isReady(): boolean {
	return highlighter !== null;
}

async function ensureLanguageLoaded(lang: string): Promise<boolean> {
	if (!(highlighter && lang)) return false;
	const loadedLangs = highlighter.getLoadedLanguages();
	if (loadedLangs.includes(lang)) return true;
	let inFlight = languageLoadPromises.get(lang);
	if (!inFlight) {
		inFlight = highlighter
			.loadLanguage(lang as Parameters<typeof highlighter.loadLanguage>[0])
			.catch(() => {
				// Unknown/unsupported language -- leave code block unhighlighted.
			})
			.finally(() => {
				languageLoadPromises.delete(lang);
			});
		languageLoadPromises.set(lang, inFlight);
	}
	await inFlight;
	return highlighter.getLoadedLanguages().includes(lang);
}

function applyShikiMarkupToCode(codeEl: HTMLElement, shikiPre: HTMLElement): void {
	const shikiCode = shikiPre.querySelector("code");
	if (!shikiCode) return;
	// Safe: codeToHtml produces deterministic syntax-highlighted markup
	// from plain-text code content. Shiki does not pass through raw user HTML.
	codeEl.textContent = "";
	while (shikiCode.firstChild) {
		codeEl.appendChild(shikiCode.firstChild);
	}
	codeEl.classList.add("shiki");
	for (const cls of shikiPre.classList) {
		if (cls !== "shiki") codeEl.classList.add(cls);
	}
}

function parseShikiPre(highlightedHtml: string): HTMLElement | null {
	const temp = document.createElement("div");
	// Safe: codeToHtml produces deterministic syntax-highlighted markup
	// from plain-text code content. The input (codeEl.textContent) is
	// already HTML-escaped by renderMarkdown(). Shiki does not pass
	// through raw user HTML -- it tokenizes and wraps in <span> tags.
	const range = document.createRange();
	const fragment = range.createContextualFragment(highlightedHtml);
	temp.appendChild(fragment);
	return temp.querySelector("pre.shiki") as HTMLElement | null;
}

async function highlightCodeElement(codeEl: HTMLElement): Promise<void> {
	if (codeEl.querySelector(".shiki") || codeEl.classList.contains("shiki")) return;
	const lang = codeEl.getAttribute("data-lang") || "";
	if (!(await ensureLanguageLoaded(lang))) return;
	const raw = codeEl.textContent || "";
	try {
		const highlightedHtml = highlighter?.codeToHtml(raw, {
			lang: lang,
			themes: {
				light: "github-light",
				dark: "github-dark",
			},
		});
		const shikiPre = parseShikiPre(highlightedHtml ?? "");
		if (!shikiPre) return;
		applyShikiMarkupToCode(codeEl, shikiPre);
	} catch (_err) {
		// Highlighting failed for this block -- leave it as plain text.
	}
}

export async function highlightCodeBlocks(containerEl: HTMLElement | null): Promise<void> {
	if (!containerEl) return;
	await initHighlighter();
	if (!highlighter) return;
	const codeEls = containerEl.querySelectorAll("pre code[data-lang]");
	for (const codeEl of codeEls) {
		await highlightCodeElement(codeEl as HTMLElement);
	}
}
