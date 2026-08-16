const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors, expectRpcOk, waitForWsConnected } = require("../helpers");

test.describe("Instrumentation settings", () => {
	test("page loads with the section heading", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/instrumentation");

		await expect(page.getByRole("heading", { name: "Instrumentation", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("shows a card for every backend", async ({ page }) => {
		await navigateAndWait(page, "/settings/instrumentation");

		// All three backends are always listed, including the disabled ones, so
		// an operator can see what is available without reading the docs first.
		await expect(page.getByText("Langfuse", { exact: true })).toBeVisible();
		await expect(page.getByText("Datadog", { exact: true })).toBeVisible();
		await expect(page.getByText("OpenTelemetry (Grafana, Honeycomb, Collector)", { exact: true })).toBeVisible();
	});

	test("explains that backends receive different data", async ({ page }) => {
		await navigateAndWait(page, "/settings/instrumentation");

		// The Langfuse/APM content asymmetry is the single most important thing
		// for an operator to understand before enabling anything, so it must be
		// on the page itself rather than only in the docs.
		await expect(page.getByText("What each backend receives", { exact: true })).toBeVisible();
	});

	test("reports instrumentation as off by default", async ({ page }) => {
		await navigateAndWait(page, "/settings/instrumentation");

		// Defaulting to on would ship conversation content to a third party
		// without the operator having chosen to.
		await expect(page.getByText(/Instrumentation is off/)).toBeVisible();
	});

	test("test-connection button is disabled while Langfuse is off", async ({ page }) => {
		await navigateAndWait(page, "/settings/instrumentation");

		const button = page.getByRole("button", { name: "Test connection" });
		await expect(button).toBeVisible();
		await expect(button).toBeDisabled();
	});

	test("status RPC responds", async ({ page }) => {
		await navigateAndWait(page, "/settings/instrumentation");
		await waitForWsConnected(page);
		await expectRpcOk(page, "instrumentation.status", {});
	});

	test("shows honest delivery status before exporters are enabled", async ({ page }) => {
		await navigateAndWait(page, "/settings/instrumentation");

		await expect(page.getByText("Delivery", { exact: true })).toBeVisible();
		await expect(page.getByText("no exporters running", { exact: true })).toBeVisible();
	});

	test("status RPC never returns the secret key", async ({ page }) => {
		await navigateAndWait(page, "/settings/instrumentation");
		await waitForWsConnected(page);
		const res = await expectRpcOk(page, "instrumentation.status", {});

		// The UI only ever needs to know whether a key is configured. Returning
		// the value would put it in every browser devtools network log.
		const serialized = JSON.stringify(res);
		expect(serialized).not.toContain('secret_key"');
		expect(serialized).toContain("secret_key_set");
	});

	test("page has no JS errors", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/instrumentation");
		expect(pageErrors).toEqual([]);
	});
});
