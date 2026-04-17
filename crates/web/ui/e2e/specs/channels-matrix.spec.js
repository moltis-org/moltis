const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Matrix channel", () => {
	test("connect button visible when matrix is offered", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await expect(addButton).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("add modal opens with OIDC as default auth mode", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();

		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Auth mode selector defaults to OIDC
		const authSelect = page.locator('select[data-field="authMode"]');
		await expect(authSelect).toBeVisible();
		await expect(authSelect).toHaveValue("oidc");

		// OIDC guidance text is shown
		await expect(page.getByText("Recommended for homeservers using Matrix Authentication Service")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("OIDC mode hides credential and user ID inputs", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// With OIDC selected (default), credential/userId inputs should not be visible
		await expect(page.locator('input[data-field="credential"]')).not.toBeVisible();
		await expect(page.locator('input[data-field="userId"]')).not.toBeVisible();

		// Homeserver input should still be visible
		const homeserverInput = page.locator('input[data-field="homeserver"]');
		await expect(homeserverInput).toBeVisible();

		// Submit button says "Authenticate with OIDC"
		await expect(page.getByRole("button", { name: "Authenticate with OIDC" })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("switching to password mode shows credential and user ID inputs", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Switch to password mode
		const authSelect = page.locator('select[data-field="authMode"]');
		await authSelect.selectOption("password");

		// Now credential/userId inputs should be visible
		await expect(page.locator('input[data-field="credential"]')).toBeVisible();
		await expect(page.locator('input[data-field="userId"]')).toBeVisible();

		// Password guidance text shown
		await expect(page.getByText("Required for encrypted Matrix chats")).toBeVisible();

		// Ownership checkbox visible for password mode
		await expect(page.getByRole("checkbox", { name: /Let Moltis own this Matrix account/i })).toBeVisible();

		// Submit button says "Connect Matrix"
		await expect(page.getByRole("button", { name: "Connect Matrix", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("switching to access_token mode shows credential but not ownership checkbox", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Switch to access_token mode
		const authSelect = page.locator('select[data-field="authMode"]');
		await authSelect.selectOption("access_token");

		// Credential should be visible, ownership checkbox should not
		await expect(page.locator('input[data-field="credential"]')).toBeVisible();
		await expect(page.getByRole("checkbox", { name: /Let Moltis own this Matrix account/i })).not.toBeVisible();

		// Access token guidance
		await expect(page.getByText("Does not support encrypted Matrix chats")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("homeserver is required for OIDC mode", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Clear the homeserver field
		const homeserverInput = page.locator('input[data-field="homeserver"]');
		await homeserverInput.clear();

		// Try to submit
		const submitButton = page.getByRole("button", { name: "Authenticate with OIDC" });
		await submitButton.click();

		// Should show error
		await expect(page.getByText("Homeserver URL is required.")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("password mode requires credential and user ID", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Switch to password mode
		const authSelect = page.locator('select[data-field="authMode"]');
		await authSelect.selectOption("password");

		// Submit without filling in credential
		const submitButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await submitButton.click();

		// Should show credential error
		await expect(page.getByText("Password is required.")).toBeVisible();

		// Fill password but not user ID
		const credentialInput = page.locator('input[data-field="credential"]');
		await credentialInput.fill("test-password");
		await submitButton.click();

		// Should show user ID error
		await expect(page.getByText("Matrix user ID is required for password login.")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("OIDC option present in auth mode dropdown with three options", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		const authSelect = page.locator('select[data-field="authMode"]');
		const options = authSelect.locator("option");
		await expect(options).toHaveCount(3);
		await expect(options.nth(0)).toHaveValue("oidc");
		await expect(options.nth(0)).toHaveText("OIDC (recommended)");
		await expect(options.nth(1)).toHaveValue("password");
		await expect(options.nth(1)).toHaveText("Password");
		await expect(options.nth(2)).toHaveValue("access_token");
		await expect(options.nth(2)).toHaveText("Access token");

		expect(pageErrors).toEqual([]);
	});

	test("encryption guidance mentions OIDC", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Encryption guidance banner should mention OIDC
		await expect(page.getByText("Encrypted chats require OIDC or Password auth")).toBeVisible();
		await expect(page.getByText("Use OIDC (recommended) or Password so Moltis creates")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("common fields visible across all auth modes", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		await expect(page.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Common fields always visible regardless of auth mode
		await expect(page.locator('select[data-field="dmPolicy"]')).toBeVisible();
		await expect(page.locator('select[data-field="roomPolicy"]')).toBeVisible();
		await expect(page.locator('select[data-field="mentionMode"]')).toBeVisible();
		await expect(page.locator('select[data-field="autoJoin"]')).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
