const { expect, test } = require("@playwright/test");
const {
	createSession,
	expectPageContentMounted,
	navigateAndWait,
	waitForWsConnected,
	watchPageErrors,
} = require("../helpers");

async function waitForWelcomeOrNoProvidersCard(page) {
	await page.waitForSelector("#welcomeCard, #noProvidersCard", {
		state: "visible",
		timeout: 10_000,
	});

	const noProvidersCard = page.locator("#noProvidersCard");
	const noProvidersVisible = await noProvidersCard.isVisible().catch(() => false);
	if (noProvidersVisible) {
		await expect(noProvidersCard.getByRole("heading", { name: "No LLMs Connected", exact: true })).toBeVisible();
		await expect(noProvidersCard.getByRole("link", { name: "Go to LLMs", exact: true })).toBeVisible();
		return null;
	}

	const welcomeCard = page.locator("#welcomeCard");
	await expect(welcomeCard).toBeVisible({ timeout: 10_000 });
	return welcomeCard;
}

async function deleteAgentByName(page, agentName) {
	await navigateAndWait(page, "/settings/agents");
	const testCard = page.locator(".backend-card").filter({ hasText: agentName });
	await expect(testCard).toBeVisible({ timeout: 10_000 });
	await testCard.getByRole("button", { name: "Delete", exact: true }).click();
	await page.locator(".provider-modal").getByRole("button", { name: "Delete", exact: true }).click();
	await expect(testCard).toHaveCount(0, { timeout: 10_000 });
}

test.describe("Agents settings page", () => {
	test("settings/agents loads and shows heading", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		await expect(page).toHaveURL(/\/settings\/agents$/);
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("main agent card is shown with Default badge", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const mainCard = page.locator(".backend-card").filter({ hasText: "Default" });
		await expect(mainCard).toBeVisible();

		// Main agent should have an "Identity Settings" button, not Edit/Delete
		await expect(mainCard.getByRole("button", { name: "Identity Settings", exact: true })).toBeVisible();
		await expect(mainCard.getByRole("button", { name: "Edit", exact: true })).toHaveCount(0);
		await expect(mainCard.getByRole("button", { name: "Delete", exact: true })).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("New Agent button opens create form", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const newBtn = page.getByRole("button", { name: "New Agent", exact: true });
		await expect(newBtn).toBeVisible();
		await newBtn.click();

		// Form should be visible with ID, Name, and Create/Cancel buttons
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();
		await expect(page.getByPlaceholder("e.g. writer, coder, researcher")).toBeVisible();
		await expect(page.getByPlaceholder("Creative Writer")).toBeVisible();
		await expect(page.getByRole("button", { name: "Create", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("create form Cancel button returns to list", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		await page.getByRole("button", { name: "Cancel", exact: true }).click();

		// Should be back to the agent list with heading and New Agent button
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "New Agent", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("create, edit, and delete an agent", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		// Create a new agent
		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		const idInput = page.getByPlaceholder("e.g. writer, coder, researcher");
		const nameInput = page.getByPlaceholder("Creative Writer");
		await idInput.fill("e2e-test-agent");
		await nameInput.fill("E2E Test Agent");
		await page.getByRole("button", { name: "Create", exact: true }).click();

		// Should return to the list and show the new agent
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible({ timeout: 10_000 });
		const agentCard = page.locator(".backend-card").filter({ hasText: "E2E Test Agent" });
		await expect(agentCard).toBeVisible();
		await expect(agentCard.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
		await expect(agentCard.getByRole("button", { name: "Delete", exact: true })).toBeVisible();

		// Edit the agent
		await agentCard.getByRole("button", { name: "Edit", exact: true }).click();
		await expect(page.getByText("Edit E2E Test Agent", { exact: true })).toBeVisible();

		const editNameInput = page.getByPlaceholder("Creative Writer");
		await editNameInput.fill("E2E Renamed Agent");
		await page.getByRole("button", { name: "Save", exact: true }).click();

		// Should return to the list with updated name
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible({ timeout: 10_000 });
		const renamedCard = page.locator(".backend-card").filter({ hasText: "E2E Renamed Agent" });
		await expect(renamedCard).toBeVisible();

		// Delete the agent
		await renamedCard.getByRole("button", { name: "Delete", exact: true }).click();
		// confirmDialog shows a custom modal — click the modal's Delete button
		await page.locator(".provider-modal").getByRole("button", { name: "Delete", exact: true }).click();

		// Agent should be removed from the list
		await expect(renamedCard).toHaveCount(0, { timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("session header agent selector switches session agent and shows sidebar indicator", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();
		await page.getByPlaceholder("e.g. writer, coder, researcher").fill("selector-test");
		await page.getByPlaceholder("Creative Writer").fill("Selector Test Agent");
		await page.getByRole("button", { name: "Create", exact: true }).click();
		await expect(page.locator(".backend-card").filter({ hasText: "Selector Test Agent" })).toBeVisible({
			timeout: 10_000,
		});

		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);

		const agentSelect = page.locator('select[title="Session agent"]');
		await expect(agentSelect).toBeEnabled({ timeout: 10_000 });
		await expect(agentSelect.locator('option[value="selector-test"]')).toBeAttached({ timeout: 10_000 });
		await agentSelect.selectOption("selector-test");
		// The controlled Preact select resets value on re-render; wait for
		// the session store to reflect the agent switch (RPC round-trip)
		// before asserting the DOM value.
		await expect
			.poll(
				async () =>
					page.evaluate(
						() => window.__moltis_stores?.sessionStore?.activeSession?.value?.agent_id,
					),
				{ timeout: 15_000 },
			)
			.toBe("selector-test");
		await expect(agentSelect).toHaveValue("selector-test");
		await expect
			.poll(async () => {
				return (
					(await page
						.locator("#sessionList .session-item.active")
						.first()
						.textContent()
						.catch(() => "")) || ""
				);
			})
			.toContain("@selector-test");

		await navigateAndWait(page, "/settings/agents");
		const testCard = page.locator(".backend-card").filter({ hasText: "Selector Test Agent" });
		await testCard.getByRole("button", { name: "Delete", exact: true }).click();
		await page.locator(".provider-modal").getByRole("button", { name: "Delete", exact: true }).click();
		await expect(testCard).toHaveCount(0, { timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("create form validates required fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		// Submit with empty fields
		await page.getByRole("button", { name: "Create", exact: true }).click();
		await expect(page.getByText("Name is required.", { exact: true })).toBeVisible();

		// Fill name but not ID
		await page.getByPlaceholder("Creative Writer").fill("Test");
		await page.getByRole("button", { name: "Create", exact: true }).click();
		await expect(page.getByText("ID is required.", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("Identity Settings button on main agent navigates to identity page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const mainCard = page.locator(".backend-card").filter({ hasText: "Default" });
		await mainCard.getByRole("button", { name: "Identity Settings", exact: true }).click();

		await expect(page).toHaveURL(/\/settings\/identity$/);
		await expectPageContentMounted(page);

		expect(pageErrors).toEqual([]);
	});
});

test.describe("Welcome card agent picker", () => {
	test("welcome card shows main agent chip and hatch button with one agent", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Navigate to a new session and wait for whichever empty chat card is valid for this runtime.
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);

		const welcomeCard = await waitForWelcomeOrNoProvidersCard(page);
		if (!welcomeCard) {
			expect(pageErrors).toEqual([]);
			return;
		}

		// Agent chips container should be visible with main chip + hatch button
		const agentsContainer = page.locator("[data-welcome-agents]");
		await expect(agentsContainer).toBeVisible();

		// The "Hatch a new agent" discovery button should be present
		await expect(agentsContainer.getByRole("button", { name: /Hatch a new agent/ })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("hatch button navigates to agents page with create form open", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.goto("/chats");
		await expectPageContentMounted(page);
		await waitForWsConnected(page);
		await createSession(page);

		const welcomeCard = await waitForWelcomeOrNoProvidersCard(page);
		if (!welcomeCard) {
			expect(pageErrors).toEqual([]);
			return;
		}

		// Click the "Hatch a new agent" button
		const hatchBtn = page.locator("[data-welcome-agents]").getByRole("button", { name: /Hatch a new agent/ });
		await expect(hatchBtn).toBeVisible();
		await hatchBtn.click();

		// Should navigate to /settings/agents/new and auto-open the create form
		await expect(page).toHaveURL(/\/settings\/agents\/new/);
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible({ timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("agent chips appear on welcome card when multiple agents exist", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const testAgentName = "Welcome Test Agent";

		// Create a second agent via the settings page
		await navigateAndWait(page, "/settings/agents");
		await waitForWsConnected(page);

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		await page.getByPlaceholder("e.g. writer, coder, researcher").fill("welcome-test");
		await page.getByPlaceholder("Creative Writer").fill(testAgentName);
		await page.getByRole("button", { name: "Create", exact: true }).click();

		// Wait for the agent to appear in the list
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible({ timeout: 10_000 });
		await expect(page.locator(".backend-card").filter({ hasText: testAgentName })).toBeVisible();

		// Navigate to chats and create a new session — welcome card should show agent chips
		await page.goto("/chats");
		await expectPageContentMounted(page);
		await createSession(page);

		const welcomeCard = await waitForWelcomeOrNoProvidersCard(page);
		if (!welcomeCard) {
			await deleteAgentByName(page, testAgentName);
			expect(pageErrors).toEqual([]);
			return;
		}

		const agentsContainer = page.locator("[data-welcome-agents]");
		await expect(agentsContainer).toBeVisible({ timeout: 10_000 });

		// Should have at least 2 chip buttons (main + the new agent)
		const chips = agentsContainer.getByRole("button");
		const chipCount = await chips.count();
		expect(chipCount).toBeGreaterThanOrEqual(2);
		await expect(agentsContainer.getByRole("button", { name: new RegExp(testAgentName) })).toBeVisible();

		// Clean up: delete the test agent
		await deleteAgentByName(page, testAgentName);

		expect(pageErrors).toEqual([]);
	});
});
