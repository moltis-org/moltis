const { expect, test } = require("../base-test");
const { createSession, expectRpcOk, navigateAndWait, waitForWsConnected } = require("../helpers");

function readClipboardMarkdown(page) {
	return page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, "\n"));
}

test.describe("Chat Markdown export", () => {
	test("assistant copy action preserves the original Markdown", async ({ page, context }) => {
		await context.grantPermissions(["clipboard-read", "clipboard-write"]);
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);
		await createSession(page);
		const sessionKey = new URL(page.url()).pathname.replace(/^\/chats\//, "").replace(/\//g, ":");

		const markdown = "**Bold** with [a link](https://example.com)\n\n```ts\nconst answer = 42;\n```";
		await expectRpcOk(page, "system-event", {
			event: "chat",
			payload: {
				sessionKey,
				state: "final",
				text: markdown,
				messageIndex: 0,
				replyMedium: "text",
				runId: "run-copy-markdown",
			},
		});

		const assistant = page.locator("#messages .msg.assistant").filter({ hasText: "Bold with a link" });
		await expect(assistant).toBeVisible();
		const footerOrder = await assistant
			.locator(":scope > .msg-model-footer, :scope > .msg-action-bar")
			.evaluateAll((elements) =>
				elements.map((element) => (element.classList.contains("msg-model-footer") ? "footer" : "actions")),
			);
		expect(footerOrder).toEqual(["footer", "actions"]);
		await assistant.getByRole("button", { name: "Copy as Markdown", exact: true }).click();

		await expect.poll(() => readClipboardMarkdown(page)).toBe(markdown);
		expect(pageErrors).toEqual([]);
	});

	test("persisted assistant response without model metadata can be copied as Markdown", async ({ page, context }) => {
		await context.grantPermissions(["clipboard-read", "clipboard-write"]);
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);
		await createSession(page);
		const sessionKey = new URL(page.url()).pathname.replace(/^\/chats\//, "").replace(/\//g, ":");
		const markdown = "Persisted **Markdown** with `code`.";

		await page.evaluate(
			({ key, content }) => {
				const cache = window.__moltis_modules?.["stores/session-history-cache"];
				const sessions = window.__moltis_modules?.sessions;
				if (!(cache?.replaceSessionHistory && sessions?.switchSession)) {
					throw new Error("session history E2E modules unavailable");
				}
				cache.replaceSessionHistory(key, [
					{
						role: "assistant",
						content,
						historyIndex: 0,
						created_at: Date.now(),
					},
				]);
				sessions.switchSession(key);
			},
			{ key: sessionKey, content: markdown },
		);

		const assistant = page.locator("#messages .msg.assistant").filter({ hasText: "Persisted Markdown" });
		await expect(assistant).toBeVisible();
		const footerOrder = await assistant
			.locator(":scope > .msg-model-footer, :scope > .msg-action-bar")
			.evaluateAll((elements) =>
				elements.map((element) => (element.classList.contains("msg-model-footer") ? "footer" : "actions")),
			);
		expect(footerOrder).toEqual(["footer", "actions"]);
		await assistant.getByRole("button", { name: "Copy as Markdown", exact: true }).click();

		await expect.poll(() => readClipboardMarkdown(page)).toBe(markdown);
		expect(pageErrors).toEqual([]);
	});

	test("session save downloads all history pages as Markdown", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const requestedPages = [];
		await page.route("**/api/sessions/main/history**", (route) => {
			const url = new URL(route.request().url());
			const cursor = url.searchParams.get("cursor");
			requestedPages.push({ cursor, limit: url.searchParams.get("limit") });
			const body =
				cursor === null
					? {
							history: [
								{
									role: "assistant",
									content: "Latest answer with **bold text**.",
									historyIndex: 2,
								},
							],
							hasMore: true,
							nextCursor: 2,
							totalMessages: 3,
						}
					: {
							history: [
								{
									role: "user",
									content: [
										{ type: "text", text: "Question with `inline code`." },
										{
											type: "image_url",
											image_url: { url: "https://example.com/a folder/image(1).png" },
										},
									],
									historyIndex: 0,
								},
								{ role: "assistant", content: "Earlier answer.", historyIndex: 1 },
							],
							hasMore: false,
							nextCursor: null,
							totalMessages: 3,
						};
			return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) });
		});

		const downloadPromise = page.waitForEvent("download");
		await page.getByRole("button", { name: "Save as Markdown", exact: true }).click();
		const download = await downloadPromise;
		expect(download.suggestedFilename()).toMatch(/\.md$/);

		const content = await (await download.createReadStream()).toArray();
		const text = Buffer.concat(content).toString("utf-8");
		expect(text).toContain("## User\n\nQuestion with `inline code`.");
		expect(text).toContain("![Image](https://example.com/a%20folder/image%281%29.png)");
		expect(text).toContain("## Assistant\n\nEarlier answer.");
		expect(text).toContain("## Assistant\n\nLatest answer with **bold text**.");
		expect(text.indexOf("Question with")).toBeLessThan(text.indexOf("Earlier answer"));
		expect(text.indexOf("Earlier answer")).toBeLessThan(text.indexOf("Latest answer"));
		expect(requestedPages).toEqual([
			{ cursor: null, limit: "500" },
			{ cursor: "2", limit: "500" },
		]);
		expect(pageErrors).toEqual([]);
	});

	test("session save does not download a partial history", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		await page.route("**/api/sessions/main/history**", (route) => {
			const cursor = new URL(route.request().url()).searchParams.get("cursor");
			const body =
				cursor === null
					? {
							history: [{ role: "assistant", content: "Latest answer.", historyIndex: 1 }],
							hasMore: true,
							nextCursor: 1,
							totalMessages: 3,
						}
					: {
							history: [{ role: "assistant", content: "Earlier answer.", historyIndex: 0 }],
							hasMore: false,
							nextCursor: null,
							totalMessages: 3,
						};
			return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) });
		});

		const downloads = [];
		page.on("download", (download) => downloads.push(download));
		const saveButton = page.getByRole("button", { name: "Save as Markdown", exact: true });
		await saveButton.click();

		await expect(page.getByText("Failed to load complete session history", { exact: true })).toBeVisible();
		await expect(saveButton).toBeEnabled();
		expect(downloads).toHaveLength(0);
		expect(pageErrors).toEqual([]);
	});
});
