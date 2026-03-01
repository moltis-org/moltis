// ── Syntax highlighting (Shiki) ────────────────────────────
//
// Lazy-loads the Shiki highlighter on app startup. Code blocks
// rendered during streaming show a language badge but no colors;
// highlighting is applied after the stream completes or when
// history messages are loaded.

import { createHighlighter } from "shiki";

var highlighter = null;

/**
 * Initialize the Shiki highlighter. Call once at app startup (fire-and-forget).
 * Safe to call multiple times — subsequent calls are no-ops.
 */
export async function initHighlighter() {
	if (highlighter) return;
	try {
		highlighter = await createHighlighter({
			themes: ["github-dark", "github-light"],
			langs: [
				"javascript",
				"typescript",
				"python",
				"rust",
				"go",
				"bash",
				"shell",
				"json",
				"html",
				"css",
				"sql",
				"yaml",
				"toml",
				"markdown",
				"c",
				"cpp",
				"java",
				"ruby",
				"php",
				"swift",
				"kotlin",
				"dockerfile",
				"xml",
				"diff",
			],
		});
	} catch (err) {
		console.warn("[shiki] failed to initialize highlighter:", err);
	}
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
export function highlightCodeBlocks(containerEl) {
	if (!highlighter || !containerEl) return;
	var codeEls = containerEl.querySelectorAll("pre code[data-lang]");
	for (var codeEl of codeEls) {
		if (codeEl.querySelector(".shiki") || codeEl.classList.contains("shiki")) continue;
		var lang = codeEl.getAttribute("data-lang") || "";
		var loadedLangs = highlighter.getLoadedLanguages();
		if (!loadedLangs.includes(lang)) continue;
		var raw = codeEl.textContent || "";
		try {
			var html = highlighter.codeToHtml(raw, {
				lang: lang,
				themes: {
					light: "github-light",
					dark: "github-dark",
				},
			});
			// Safe: codeToHtml produces deterministic syntax-highlighted markup
			// from plain-text code content. The input (codeEl.textContent) is
			// already HTML-escaped by renderMarkdown(). Shiki does not pass
			// through raw user HTML — it tokenizes and wraps in <span> tags.
			var temp = document.createElement("div");
			temp.innerHTML = html; // eslint-disable-line no-unsanitized/property
			var shikiPre = temp.querySelector("pre.shiki");
			if (shikiPre) {
				// Copy Shiki's style attribute to the parent <pre> for theming
				var parentPre = codeEl.parentElement;
				if (parentPre && parentPre.tagName === "PRE") {
					parentPre.style.cssText = shikiPre.style.cssText;
				}
				var shikiCode = shikiPre.querySelector("code");
				if (shikiCode) {
					codeEl.innerHTML = shikiCode.innerHTML; // eslint-disable-line no-unsanitized/property
					codeEl.classList.add("shiki");
					// Copy shiki theme classes onto code element
					for (var cls of shikiPre.classList) {
						if (cls !== "shiki") codeEl.classList.add(cls);
					}
				}
			}
		} catch (_err) {
			// Highlighting failed for this block — leave it as plain text.
		}
	}
}
