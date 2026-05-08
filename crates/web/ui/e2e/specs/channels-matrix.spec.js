const { expect, test } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("Matrix channel", () => {
	function matrixModal(page) {
		return page.locator(".modal-box");
	}

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

		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Auth mode selector defaults to OIDC
		const authSelect = modal.locator('select[data-field="authMode"]');
		await expect(authSelect).toBeVisible();
		await expect(authSelect).toHaveValue("oidc");

		// OIDC guidance text is shown
		await expect(
			modal.getByText("Recommended for homeservers using Matrix Authentication Service", { exact: false }),
		).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("OIDC mode hides credential and user ID inputs", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// With OIDC selected (default), credential/userId inputs should not be visible
		await expect(modal.locator('input[data-field="credential"]')).not.toBeVisible();
		await expect(modal.locator('input[data-field="userId"]')).not.toBeVisible();

		// Homeserver input should still be visible
		const homeserverInput = modal.locator('input[data-field="homeserver"]');
		await expect(homeserverInput).toBeVisible();

		// Submit button says "Authenticate with OIDC"
		await expect(modal.getByRole("button", { name: "Authenticate with OIDC", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("switching to password mode shows credential and user ID inputs", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Switch to password mode
		const authSelect = modal.locator('select[data-field="authMode"]');
		await authSelect.selectOption("password");

		// Now credential/userId inputs should be visible
		await expect(modal.locator('input[data-field="credential"]')).toBeVisible();
		await expect(modal.locator('input[data-field="userId"]')).toBeVisible();

		// Password guidance text shown
		await expect(modal.getByText("Required for encrypted Matrix chats", { exact: false })).toBeVisible();

		// Ownership checkbox visible for password mode
		await expect(modal.getByRole("checkbox", { name: /Let Moltis own this Matrix account/i })).toBeVisible();

		// Submit button says "Connect Matrix"
		await expect(modal.getByRole("button", { name: "Connect Matrix", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("switching to access_token mode shows credential but not ownership checkbox", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Switch to access_token mode
		const authSelect = modal.locator('select[data-field="authMode"]');
		await authSelect.selectOption("access_token");

		// Credential should be visible, ownership checkbox should not
		await expect(modal.locator('input[data-field="credential"]')).toBeVisible();
		await expect(modal.getByRole("checkbox", { name: /Let Moltis own this Matrix account/i })).not.toBeVisible();

		// Access token guidance
		await expect(
			modal.getByText(
				"Does not support encrypted Matrix chats. Access tokens authenticate an existing Matrix session",
				{
					exact: false,
				},
			),
		).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("homeserver is required for OIDC mode", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Clear the homeserver field
		const homeserverInput = modal.locator('input[data-field="homeserver"]');
		await homeserverInput.clear();

		// Try to submit
		const submitButton = modal.getByRole("button", { name: "Authenticate with OIDC", exact: true });
		await submitButton.click();

		// Should show error
		await expect(modal.getByText("Homeserver URL is required.", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("password mode requires credential and user ID", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Switch to password mode
		const authSelect = modal.locator('select[data-field="authMode"]');
		await authSelect.selectOption("password");

		// Submit without filling in credential
		const submitButton = modal.getByRole("button", { name: "Connect Matrix", exact: true });
		await submitButton.click();

		// Should show credential error
		await expect(modal.getByText("Password is required.", { exact: true })).toBeVisible();

		// Fill password but not user ID
		const credentialInput = modal.locator('input[data-field="credential"]');
		await credentialInput.fill("test-password");
		await submitButton.click();

		// Should show user ID error
		await expect(modal.getByText("Matrix user ID is required for password login.", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("OIDC option present in auth mode dropdown with three options", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		const authSelect = modal.locator('select[data-field="authMode"]');
		const options = authSelect.locator("option");
		await expect(options).toHaveCount(3);
		await expect(options.nth(0)).toHaveAttribute("value", "oidc");
		await expect(options.nth(0)).toHaveText("OIDC (recommended)");
		await expect(options.nth(1)).toHaveAttribute("value", "password");
		await expect(options.nth(1)).toHaveText("Password");
		await expect(options.nth(2)).toHaveAttribute("value", "access_token");
		await expect(options.nth(2)).toHaveText("Access token");

		expect(pageErrors).toEqual([]);
	});

	test("encryption guidance mentions OIDC", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Encryption guidance banner should mention OIDC
		await expect(
			modal.getByText("Encrypted Matrix chats require OIDC or Password auth", { exact: false }),
		).toBeVisible();
		await expect(
			modal.getByText("Use OIDC (recommended) or Password so Moltis creates", { exact: false }),
		).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("common fields visible across all auth modes", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/channels");
		await waitForWsConnected(page);

		const addButton = page.getByRole("button", { name: "Connect Matrix", exact: true });
		await addButton.click();
		const modal = matrixModal(page);
		await expect(modal.getByRole("heading", { name: "Connect Matrix", exact: true })).toBeVisible();

		// Common fields always visible regardless of auth mode
		await expect(modal.locator('select[data-field="dmPolicy"]')).toBeVisible();
		await expect(modal.locator('select[data-field="roomPolicy"]')).toBeVisible();
		await expect(modal.locator('select[data-field="mentionMode"]')).toBeVisible();
		await expect(modal.locator('select[data-field="autoJoin"]')).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
