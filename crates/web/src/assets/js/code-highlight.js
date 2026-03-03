// ── Syntax highlighting (Shiki) ────────────────────────────
//
// Lazy-loads the Shiki highlighter on app startup. Code blocks
// rendered during streaming show a language badge but no colors;
// highlighting is applied after the stream completes or when
// history messages are loaded.

var highlighter = null;
var highlighterInitPromise = null;
var languageLoadPromises = new Map();

/**
 * Initialize the Shiki highlighter. Call once at app startup (fire-and-forget).
 * Safe to call multiple times — subsequent calls are no-ops.
 */
export async function initHighlighter() {
	if (highlighter) return highlighter;
	if (highlighterInitPromise) {
		await highlighterInitPromise;
		return highlighter;
	}
	highlighterInitPromise = (async () => {
		try {
			var shiki = await import("shiki");
			// Load only themes at startup; grammars are loaded on demand per language.
			highlighter = await shiki.createHighlighter({
				themes: ["github-dark", "github-light"],
			});
		} catch (err) {
			console.warn("[shiki] failed to initialize highlighter:", err);
		}
	})();
	await highlighterInitPromise;
	return highlighter;
}

/** Returns whether the highlighter has finished loading. */
export function isReady() {
	return highlighter !== null;
}

/**
 * Find all unhighlighted `<pre><code[data-lang]>` elements inside
 * `containerEl` and replace their content with Shiki-highlighted HTML.
 *
 * Skips blocks that have already been highlighted (`.shiki` class present).
 * If the highlighter hasn't loaded yet, this is a silent no-op.
 *
 * @param {HTMLElement} containerEl
 */
async function ensureLanguageLoaded(lang) {
	if (!(highlighter && lang)) return false;
	var loadedLangs = highlighter.getLoadedLanguages();
	if (loadedLangs.includes(lang)) return true;
	var inFlight = languageLoadPromises.get(lang);
	if (!inFlight) {
		inFlight = highlighter
			.loadLanguage(lang)
			.catch(() => {
				// Unknown/unsupported language — leave code block unhighlighted.
			})
			.finally(() => {
				languageLoadPromises.delete(lang);
			});
		languageLoadPromises.set(lang, inFlight);
	}
	await inFlight;
	return highlighter.getLoadedLanguages().includes(lang);
}

function applyShikiStylesToPre(codeEl, shikiPre) {
	var parentPre = codeEl.parentElement;
	if (!(parentPre && parentPre.tagName === "PRE")) return;
	// Copy Shiki's style attribute to the parent <pre> for theming.
	parentPre.style.cssText = shikiPre.style.cssText;
}

function applyShikiMarkupToCode(codeEl, shikiPre) {
	var shikiCode = shikiPre.querySelector("code");
	if (!shikiCode) return;
	codeEl.innerHTML = shikiCode.innerHTML; // eslint-disable-line no-unsanitized/property
	codeEl.classList.add("shiki");
	for (var cls of shikiPre.classList) {
		if (cls !== "shiki") codeEl.classList.add(cls);
	}
}

function parseShikiPre(highlightedHtml) {
	var temp = document.createElement("div");
	// Safe: codeToHtml produces deterministic syntax-highlighted markup
	// from plain-text code content. The input (codeEl.textContent) is
	// already HTML-escaped by renderMarkdown(). Shiki does not pass
	// through raw user HTML — it tokenizes and wraps in <span> tags.
	temp.innerHTML = highlightedHtml; // eslint-disable-line no-unsanitized/property
	return temp.querySelector("pre.shiki");
}

async function highlightCodeElement(codeEl) {
	if (codeEl.querySelector(".shiki") || codeEl.classList.contains("shiki")) return;
	var lang = codeEl.getAttribute("data-lang") || "";
	if (!(await ensureLanguageLoaded(lang))) return;
	var raw = codeEl.textContent || "";
	try {
		var highlightedHtml = highlighter.codeToHtml(raw, {
			lang: lang,
			themes: {
				light: "github-light",
				dark: "github-dark",
			},
		});
		var shikiPre = parseShikiPre(highlightedHtml);
		if (!shikiPre) return;
		applyShikiStylesToPre(codeEl, shikiPre);
		applyShikiMarkupToCode(codeEl, shikiPre);
	} catch (_err) {
		// Highlighting failed for this block — leave it as plain text.
	}
}

export async function highlightCodeBlocks(containerEl) {
	if (!containerEl) return;
	await initHighlighter();
	if (!highlighter) return;
	var codeEls = containerEl.querySelectorAll("pre code[data-lang]");
	for (var codeEl of codeEls) {
		await highlightCodeElement(codeEl);
	}
}
