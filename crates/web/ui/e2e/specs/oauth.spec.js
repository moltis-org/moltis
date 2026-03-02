const { expect, test } = require("@playwright/test");
const fs = require("node:fs");
const path = require("node:path");
const { navigateAndWait, watchPageErrors, expectPageContentMounted, waitForWsConnected } = require("../helpers");

// Resolve paths relative to the repo root
var repoRoot = path.resolve(__dirname, "../../../../..");
var runtimeRoot = process.env.MOLTIS_E2E_OAUTH_RUNTIME_DIR || path.join(repoRoot, "target/e2e-runtime-oauth");
var runtimeConfigDir = path.join(runtimeRoot, "config");
var runtimeHomeConfigDir = path.join(runtimeRoot, "home", ".config", "moltis");

function getMockPort() {
	var portFile = path.join(runtimeRoot, "mock-oauth-port");
	try {
		return parseInt(fs.readFileSync(portFile, "utf8").trim(), 10);
	} catch {
		return null;
	}
}

function mockUrl(mockPort, pathname) {
	return `http://127.0.0.1:${mockPort}${pathname}`;
}

async function resetMockServer(mockPort) {
	var res = await fetch(mockUrl(mockPort, "/reset"), { method: "POST" });
	return res.ok;
}

async function getMockCalls(mockPort) {
	var res = await fetch(mockUrl(mockPort, "/calls"));
	return res.json();
}

async function configureMock(mockPort, options) {
	var body = new URLSearchParams(options);
	var res = await fetch(mockUrl(mockPort, "/config"), { method: "POST", body });
	return res.ok;
}

function removeIfExists(filePath) {
	try {
		fs.rmSync(filePath, { force: true });
	} catch {
		// Best-effort cleanup only.
	}
}

function resetRuntimeAuthState() {
	removeIfExists(path.join(runtimeConfigDir, "oauth_tokens.json"));
	removeIfExists(path.join(runtimeConfigDir, "provider_keys.json"));
	removeIfExists(path.join(runtimeHomeConfigDir, "oauth_tokens.json"));
	removeIfExists(path.join(runtimeHomeConfigDir, "provider_keys.json"));
}

async function openProviderPicker(page) {
	await waitForWsConnected(page);
	await page.getByRole("button", { name: "Add LLM" }).click();
	var codexCard = page.locator("#providerModalBody .provider-item").filter({ hasText: "OpenAI Codex" }).first();
	await expect(codexCard).toBeVisible();
	return codexCard;
}

test.describe("OAuth provider connection", () => {
	var mockPort;

	test.beforeAll(() => {
		mockPort = getMockPort();
		if (!mockPort) {
			throw new Error(`Could not read mock OAuth server port from ${path.join(runtimeRoot, "mock-oauth-port")}`);
		}
	});

	test.beforeEach(async () => {
		resetRuntimeAuthState();
		if (mockPort) await resetMockServer(mockPort);
	});

	test("provider list shows OAuth providers", async ({ page }) => {
		var pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");

		await expect(page.getByRole("heading", { name: "LLMs" })).toBeVisible();

		// Click "Add LLM" to see available providers.
		var codexCard = await openProviderPicker(page);
		await expect(codexCard.locator(".provider-item-badge.oauth")).toHaveText("OAuth");

		expect(pageErrors).toEqual([]);
	});

	test("OAuth PKCE flow completes successfully", async ({ page, context }) => {
		var pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");

		// Click "Add LLM" to open provider modal.
		var codexCard = await openProviderPicker(page);

		// Click on OpenAI Codex to start the OAuth flow.
		await codexCard.click();
		await expect(page.getByRole("button", { name: "Connect" })).toBeVisible();

		// Listen for the popup that opens the OAuth auth URL.
		var popupPromise = context.waitForEvent("page", { timeout: 10_000 });

		// Click "Connect" to start the OAuth flow
		await page.getByRole("button", { name: "Connect" }).click();

		// The popup navigates to the mock server /authorize, which redirects
		// back to the gateway's /auth/callback with code + state. The gateway
		// exchanges the code and stores tokens.
		var popup = await popupPromise;
		// Callback success page may auto-close very quickly.
		if (!popup.isClosed()) {
			await popup.waitForEvent("close", { timeout: 10_000 }).catch(() => {
				// Continue: main-page polling and mock call assertions below verify success.
			});
		}

		// Back in the main page, wait for the polling to detect the authenticated state.
		// The UI should either show "connected" or transition to a model selector.
		await expect(page.getByText(/connected successfully|Select Model/i)).toBeVisible({ timeout: 15_000 });

		// Verify mock server received the expected calls
		var calls = await getMockCalls(mockPort);
		var authorizeCalls = calls.filter((c) => c.path === "/authorize");
		var tokenCalls = calls.filter((c) => c.path === "/token");

		expect(authorizeCalls.length).toBe(1);
		expect(tokenCalls.length).toBe(1);

		// Verify the authorize call had PKCE params
		var authCall = authorizeCalls[0];
		expect(authCall.query.code_challenge).toBeTruthy();
		expect(authCall.query.code_challenge_method).toBe("S256");
		expect(authCall.query.state).toBeTruthy();
		expect(authCall.query.client_id).toBe("test-client-id");

		expect(pageErrors).toEqual([]);
	});

	test("OAuth state mismatch is rejected", async ({ page }) => {
		var _pageErrors = watchPageErrors(page);

		// Navigate directly to the callback with a bogus state
		var response = await page.goto("/auth/callback?code=fake-code&state=wrong-state");

		// The gateway should return a 400 error
		expect(response.status()).toBe(400);
		await expect(page.getByText("Authentication failed")).toBeVisible();

		// Allow the expected page error (no JS error from our code, just the error page)
		// pageErrors may be empty since the error page is a simple HTML page
	});

	test("disconnect removes provider tokens", async ({ page, context }) => {
		var pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");

		// First, connect the provider.
		var codexCard = await openProviderPicker(page);
		await codexCard.click();
		await expect(page.getByRole("button", { name: "Connect" })).toBeVisible();

		var popupPromise = context.waitForEvent("page", { timeout: 10_000 });
		await page.getByRole("button", { name: "Connect" }).click();
		var popup = await popupPromise;
		if (!popup.isClosed()) {
			await popup.waitForEvent("close", { timeout: 10_000 }).catch(() => {
				// Continue: main-page polling and follow-up assertions verify success.
			});
		}

		// Wait for connection to complete.
		await expect(page.getByText(/connected successfully|Select Model/i)).toBeVisible({ timeout: 15_000 });

		// Close the modal if a model picker is still open.
		var modalClose = page.locator("#providerModalClose");
		if (await modalClose.isVisible().catch(() => false)) {
			await modalClose.click();
		}
		await expect(page.locator("#providerModal")).toHaveClass(/hidden/);

		// Navigate to providers page to see the connected provider.
		await navigateAndWait(page, "/settings/providers");
		await expectPageContentMounted(page);

		// The provider should appear in the list. Delete it and confirm.
		var configuredList = page.locator("#pageContent");
		var codexHeading = configuredList.getByRole("heading", { name: "OpenAI Codex" });
		await expect(codexHeading).toBeVisible();
		var codexSection = codexHeading.locator("xpath=ancestor::div[contains(@class, 'max-w-form')]").first();
		await codexSection.getByRole("button", { name: "Delete" }).click();
		await page.getByRole("button", { name: "Confirm" }).click();
		await expect(configuredList.getByRole("heading", { name: "OpenAI Codex" })).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("token exchange failure shows error", async ({ page, context }) => {
		var pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/providers");

		// Configure mock to fail token exchange
		await configureMock(mockPort, { token_should_fail: "true" });

		// Start OAuth flow.
		var codexCard = await openProviderPicker(page);
		await codexCard.click();
		await expect(page.getByRole("button", { name: "Connect" })).toBeVisible();

		var popupPromise = context.waitForEvent("page", { timeout: 10_000 });
		await page.getByRole("button", { name: "Connect" }).click();

		var popup = await popupPromise;
		await popup.waitForLoadState("domcontentloaded");

		// The callback should fail because the mock /token returns 400
		await expect(popup.getByText("Authentication failed")).toBeVisible({ timeout: 10_000 });

		// The main page should still show the connect button after timeout/failure
		// (poll will time out since tokens were never stored)
		// We verify the mock received both calls
		var calls = await getMockCalls(mockPort);
		var authorizeCalls = calls.filter((c) => c.path === "/authorize");
		var tokenCalls = calls.filter((c) => c.path === "/token");

		expect(authorizeCalls.length).toBe(1);
		expect(tokenCalls.length).toBe(1);

		expect(pageErrors).toEqual([]);
	});

	test("missing callback code returns 400", async ({ page }) => {
		// Navigate to callback with no code parameter
		var response = await page.goto("/auth/callback?state=some-state");
		expect(response.status()).toBe(400);
		await expect(page.getByText("Missing authorization code")).toBeVisible();
	});

	test("missing callback state returns 400", async ({ page }) => {
		// Navigate to callback with no state parameter
		var response = await page.goto("/auth/callback?code=some-code");
		expect(response.status()).toBe(400);
		await expect(page.getByText("Missing OAuth state")).toBeVisible();
	});
});
