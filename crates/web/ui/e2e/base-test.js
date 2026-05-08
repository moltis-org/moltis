// Shared Playwright test fixture with automatic error-context capture.
//
// Every spec file should import { test, expect } from this module instead of
// from "@playwright/test".  On failure the fixture attaches a markdown
// snapshot of every open page (URL, title, visible text) so CI logs contain
// enough context to diagnose failures without downloading trace artifacts.

const { test: base, expect } = require("@playwright/test");

var test = base.extend({
	page: async ({ page, context }, use, testInfo) => {
		await use(page);

		if (testInfo.status !== testInfo.expectedStatus) {
			var pages = context.pages();
			var parts = [];

			for (var i = 0; i < pages.length; i++) {
				var p = pages[i];
				try {
					if (p.isClosed()) {
						parts.push(`### Page ${i + 1}: (closed)`);
						continue;
					}
					var url = p.url();
					var title = await p.title().catch(() => "(unknown)");
					var text = await p
						.evaluate(() => document.body?.innerText?.slice(0, 3000) || "")
						.catch(() => "(unavailable)");
					parts.push(`### Page ${i + 1}: ${title}\n- **URL**: ${url}\n\n\`\`\`\n${text}\n\`\`\``);

					// Capture full-browser screenshot of each open page on failure.
					// If the page is at about:blank (goto never completed), try
					// navigating to the base URL first so we can see if the server
					// is alive and what state the app is in.
					if (url === "about:blank") {
						var baseURL = testInfo.project.use?.baseURL || "http://127.0.0.1";
						await p.goto(baseURL, { waitUntil: "load", timeout: 10_000 }).catch(() => {});
					}
					var screenshot = await p.screenshot({ fullPage: true }).catch(() => null);
					if (screenshot) {
						await testInfo.attach(`failure-screenshot-page-${i + 1}`, {
							body: screenshot,
							contentType: "image/png",
						});
					}
				} catch {
					parts.push(`### Page ${i + 1}: (error reading page)`);
				}
			}

			var md = ["## Error Context", "", `**Test**: ${testInfo.title}`, `**Status**: ${testInfo.status}`, ""]
				.concat(parts)
				.join("\n");

			await testInfo.attach("error-context", {
				body: Buffer.from(md, "utf-8"),
				contentType: "text/markdown",
			});
		}
	},
});

module.exports = { test, expect };
