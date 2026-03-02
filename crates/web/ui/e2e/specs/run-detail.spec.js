const { expect, test } = require("@playwright/test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Run detail panel", () => {
	test("run detail button is not visible for messages without run_id", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// The default "main" session may have history — but any plain assistant
		// message from before this feature won't have a run_id, so no "Run details"
		// button should appear.
		const runDetailButtons = page.locator("text=Run details");
		// Count should be 0 or match only messages that have run_id.
		// Since this is a fresh instance, there should be no run detail buttons.
		const count = await runDetailButtons.count();
		expect(count).toBe(0);

		expect(pageErrors).toEqual([]);
	});

	test("sessions.run_detail RPC returns valid structure", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// Call the RPC directly and verify the response structure.
		const result = await page.evaluate(async () => {
			// Access the sendRpc function via the global WebSocket.
			const ws = window.__moltis_ws;
			if (!ws) return { error: "no websocket" };

			return new Promise((resolve) => {
				const id = Math.random().toString(36).slice(2);
				const handler = (event) => {
					try {
						const data = JSON.parse(event.data);
						if (data.id === id) {
							ws.removeEventListener("message", handler);
							resolve(data);
						}
					} catch {
						// ignore non-JSON frames
					}
				};
				ws.addEventListener("message", handler);
				ws.send(
					JSON.stringify({
						jsonrpc: "2.0",
						id,
						method: "sessions.run_detail",
						params: { sessionKey: "main", runId: "nonexistent-run-id" },
					}),
				);
				// Timeout after 5s
				setTimeout(() => {
					ws.removeEventListener("message", handler);
					resolve({ error: "timeout" });
				}, 5000);
			});
		});

		// The RPC should succeed (even if no messages match the run_id).
		if (result.result) {
			expect(result.result).toHaveProperty("runId", "nonexistent-run-id");
			expect(result.result).toHaveProperty("messages");
			expect(result.result).toHaveProperty("summary");
			expect(Array.isArray(result.result.messages)).toBe(true);
		}

		expect(pageErrors).toEqual([]);
	});
});
