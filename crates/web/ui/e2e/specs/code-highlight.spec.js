const { expect, test } = require("../base-test");
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
			var markdown = "Here is some code:\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
			var existing = document.getElementById("e2e-code-highlight-fixture");
			if (existing) existing.remove();
			var fixture = document.createElement("div");
			fixture.id = "e2e-code-highlight-fixture";
			fixture.className = "msg assistant";
			fixture.innerHTML = helpers.renderMarkdown(markdown); // eslint-disable-line no-unsanitized/property
			document.body.appendChild(fixture);
		});

		// Verify data-lang attribute is present
		var codeEl = page.locator("#e2e-code-highlight-fixture pre code[data-lang='rust']");
		await expect(codeEl).toBeVisible({ timeout: 5000 });

		// Verify language badge is displayed
		var badge = page.locator("#e2e-code-highlight-fixture .code-lang-badge");
		await expect(badge).toBeVisible();
		await expect(badge).toHaveText("rust");

		// Verify the pre has the code-block class
		var pre = page.locator("#e2e-code-highlight-fixture pre.code-block");
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
			var codeHighlight = await import(`${prefix}js/code-highlight.js`);
			var markdown = "```javascript\nconst x = 42;\n```";
			var existing = document.getElementById("e2e-shiki-fixture");
			if (existing) existing.remove();
			var fixture = document.createElement("div");
			fixture.id = "e2e-shiki-fixture";
			fixture.className = "msg assistant";
			fixture.innerHTML = helpers.renderMarkdown(markdown); // eslint-disable-line no-unsanitized/property
			document.body.appendChild(fixture);
			codeHighlight.highlightCodeBlocks(fixture);
		});

		// Verify Shiki classes are applied
		var shikiCode = page.locator("#e2e-shiki-fixture code.shiki");
		await expect(shikiCode).toBeVisible({ timeout: 5000 });

		// Verify spans with style attributes are present (Shiki token coloring)
		var coloredSpan = page.locator("#e2e-shiki-fixture code.shiki span[style]");
		await expect(coloredSpan.first()).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
