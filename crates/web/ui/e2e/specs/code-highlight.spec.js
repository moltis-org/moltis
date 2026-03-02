const { expect, test } = require("@playwright/test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Code block syntax highlighting", () => {
	test("code blocks get data-lang attribute and language badge", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Inject a message with a code block into the chat via renderMarkdown
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) throw new Error("app module script not found");
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var helpers = await import(`${prefix}js/helpers.js`);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var markdown = "Here is some code:\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
			chatUi.chatAddMsg("assistant", helpers.renderMarkdown(markdown), true);
		});

		// Verify data-lang attribute is present
		var codeEl = page.locator(".msg.assistant pre code[data-lang='rust']");
		await expect(codeEl).toBeVisible({ timeout: 5000 });

		// Verify language badge is displayed
		var badge = page.locator(".msg.assistant .code-lang-badge");
		await expect(badge).toBeVisible();
		await expect(badge).toHaveText("rust");

		// Verify the pre has the code-block class
		var pre = page.locator(".msg.assistant pre.code-block");
		await expect(pre).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("shiki highlighter applies syntax classes after init", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Wait for the shiki highlighter to initialize
		await expect
			.poll(
				async () => {
					return page.evaluate(async () => {
						var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
						if (!appScript) return false;
						var appUrl = new URL(appScript.src, window.location.origin);
						var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
						var codeHighlight = await import(`${prefix}js/code-highlight.js`);
						return codeHighlight.isReady();
					});
				},
				{ timeout: 15_000 },
			)
			.toBe(true);

		// Add a message and highlight it
		await page.evaluate(async () => {
			var appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			var appUrl = new URL(appScript.src, window.location.origin);
			var prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			var helpers = await import(`${prefix}js/helpers.js`);
			var chatUi = await import(`${prefix}js/chat-ui.js`);
			var codeHighlight = await import(`${prefix}js/code-highlight.js`);
			var markdown = "```javascript\nconst x = 42;\n```";
			var el = chatUi.chatAddMsg("assistant", helpers.renderMarkdown(markdown), true);
			if (el) codeHighlight.highlightCodeBlocks(el);
		});

		// Verify Shiki classes are applied
		var shikiCode = page.locator(".msg.assistant code.shiki");
		await expect(shikiCode).toBeVisible({ timeout: 5000 });

		// Verify spans with style attributes are present (Shiki token coloring)
		var coloredSpan = page.locator(".msg.assistant code.shiki span[style]");
		await expect(coloredSpan.first()).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
