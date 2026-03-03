// Shared Playwright test fixture with automatic error-context capture.
//
// Every spec file should import { test, expect } from this module instead of
// from "@playwright/test".  On failure the fixture attaches a markdown
// snapshot of every open page (URL, title, visible text) so CI logs contain
// enough context to diagnose failures without downloading trace artifacts.

const { test: base, expect } = require("@playwright/test");

var test = base.extend({
	// biome-ignore lint/correctness/noUnusedVariables: Playwright fixture signature requires destructured params even when unused
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
				} catch {
					parts.push(`### Page ${i + 1}: (error reading page)`);
				}
			}

			var md = ["## Error Context", "", `**Test**: ${testInfo.title}`, `**Status**: ${testInfo.status}`, ""].concat(parts).join("\n");

			await testInfo.attach("error-context", {
				body: Buffer.from(md, "utf-8"),
				contentType: "text/markdown",
			});
		}
	},
});

module.exports = { test, expect };
