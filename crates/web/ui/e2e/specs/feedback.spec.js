const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors, expectRpcOk, sendRpcFromPage, waitForWsConnected } = require("../helpers");

test.describe("Reaction feedback", () => {
	test("status RPC responds", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const response = await expectRpcOk(page, "feedback.status", {});
		expect(typeof response.payload.enabled).toBe("boolean");
		expect(typeof response.payload.retention_days).toBe("number");
	});

	test("reports feedback as unavailable while instrumentation is off", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// A thumb that goes nowhere is worse than no thumb, so the control is
		// gated on something actually collecting the score.
		const response = await expectRpcOk(page, "feedback.status", {});
		expect(response.payload.enabled).toBe(false);
	});

	test("submitting without a session key is rejected", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const response = await sendRpcFromPage(page, "feedback.submit", {
			messageId: "run-1",
			signal: "positive",
		});

		expect(response.ok).toBe(false);
	});

	test("submitting an unknown signal is rejected", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		// The wire format must not be able to smuggle an arbitrary value into
		// the score.
		const response = await sendRpcFromPage(page, "feedback.submit", {
			sessionKey: "agent:main:main",
			messageId: "run-1",
			signal: "0.7",
		});

		expect(response.ok).toBe(false);
	});

	test("a thumb on an unlinked message reports why rather than failing silently", async ({ page }) => {
		await navigateAndWait(page, "/");
		await waitForWsConnected(page);

		const response = await sendRpcFromPage(page, "feedback.submit", {
			sessionKey: "agent:main:main",
			messageId: "run-that-never-existed",
			signal: "positive",
		});

		// The RPC itself succeeds; the outcome explains that the turn is not
		// attributable, which is not an error the user can retry away.
		expect(response.ok).toBe(true);
		expect(response.payload.ok).toBe(false);
		expect(["unknown_message", "disabled"]).toContain(response.payload.outcome);
	});

	test("chat page loads with no JS errors when feedback is unavailable", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/");

		// The action bar appends thumbs asynchronously; a failed or disabled
		// status check must not surface as a page error.
		await expect(page.locator("#chatInput")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});
});
