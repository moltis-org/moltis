const { expect, test } = require("@playwright/test");
const { watchPageErrors } = require("../helpers");

const LLM_STEP_HEADING = /^(Add LLMs|Add providers)$/;
const OPENAI_API_KEY = process.env.MOLTIS_E2E_OPENAI_API_KEY || process.env.OPENAI_API_KEY || "";

function isVisible(locator) {
	return locator.isVisible().catch(() => false);
}

async function clickFirstVisibleButton(page, roleQuery) {
	const buttons = page.locator(".onboarding-card").getByRole("button", roleQuery);
	const count = await buttons.count();
	for (let i = 0; i < count; i++) {
		const button = buttons.nth(i);
		if (!(await isVisible(button))) continue;
		await button.click();
		return true;
	}
	return false;
}

async function waitForOnboardingStepLoaded(page) {
	await expect(page.locator(".onboarding-card")).toBeVisible();
	await expect(page.getByText("Loading…")).toHaveCount(0, { timeout: 10_000 });
}

async function maybeSkipAuth(page) {
	const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
	if (!(await isVisible(authHeading))) return false;

	return clickFirstVisibleButton(page, { name: /skip/i });
}

async function maybeCompleteIdentity(page) {
	const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
	if (!(await isVisible(identityHeading))) return false;

	await page.getByPlaceholder("e.g. Alice").fill("E2E User");
	const agentInput = page.getByPlaceholder("e.g. Rex");
	if (await isVisible(agentInput)) {
		await agentInput.fill("E2E Bot");
	}
	await page.getByRole("button", { name: "Continue", exact: true }).click();
	return true;
}

async function maybeSkipOpenClawImport(page) {
	const importHeading = page.getByRole("heading", { name: "Import from OpenClaw", exact: true });
	if (!(await isVisible(importHeading))) return false;

	return clickFirstVisibleButton(page, { name: /^Skip/ });
}

async function moveToLlmStep(page) {
	const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
	for (let i = 0; i < 40; i++) {
		await waitForOnboardingStepLoaded(page);
		if (await isVisible(llmHeading)) return;

		if (await maybeSkipAuth(page)) continue;
		if (await maybeSkipOpenClawImport(page)) continue;
		if (await maybeCompleteIdentity(page)) continue;

		const backButton = page.locator(".onboarding-card").getByRole("button", { name: "Back", exact: true }).first();
		if (await isVisible(backButton)) {
			await backButton.click();
			continue;
		}

		await page.waitForTimeout(500);
	}

	await expect(llmHeading).toBeVisible({ timeout: 45_000 });
}

test.describe("Onboarding OpenAI provider", () => {
	test.describe.configure({ mode: "serial" });

	test.skip(!OPENAI_API_KEY, "requires OPENAI_API_KEY or MOLTIS_E2E_OPENAI_API_KEY");

	test("detected OpenAI choose model opens selector without asking for API key again", async ({ page }) => {
		test.setTimeout(90_000);
		const pageErrors = watchPageErrors(page);

		await page.goto("/onboarding");
		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);

		if (/^\/chats\//.test(new URL(page.url()).pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		await moveToLlmStep(page);
		await expect(page.getByRole("heading", { name: LLM_STEP_HEADING })).toBeVisible();
		await expect(page.getByText("Detected LLM providers", { exact: true })).toBeVisible();

		const openaiRow = page
			.locator(".onboarding-card .rounded-md.border")
			.filter({ has: page.getByText("OpenAI", { exact: true }) })
			.filter({ has: page.getByText("API Key", { exact: true }) })
			.first();

		await expect(openaiRow).toBeVisible();
		await expect(openaiRow.locator(".provider-item-badge.configured")).toBeVisible();
		await expect(openaiRow.getByRole("button", { name: "Choose Model", exact: true })).toBeVisible();

		await openaiRow.getByRole("button", { name: "Choose Model", exact: true }).click();

		await expect(openaiRow.getByText("Select preferred models", { exact: true })).toBeVisible({ timeout: 45_000 });
		await expect(openaiRow.locator(".model-card").first()).toBeVisible({ timeout: 45_000 });
		await expect(openaiRow.locator("input[type='password']")).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("continue saves selected OpenAI models during onboarding", async ({ page }) => {
		test.setTimeout(120_000);
		const pageErrors = watchPageErrors(page);

		await page.goto("/onboarding");
		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);

		if (/^\/chats\//.test(new URL(page.url()).pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		await moveToLlmStep(page);
		await expect(page.getByRole("heading", { name: LLM_STEP_HEADING })).toBeVisible();
		await expect(page.getByText("Detected LLM providers", { exact: true })).toBeVisible();

		const openaiRow = page
			.locator(".onboarding-card .rounded-md.border")
			.filter({ has: page.getByText("OpenAI", { exact: true }) })
			.filter({ has: page.getByText("API Key", { exact: true }) })
			.first();

		await expect(openaiRow).toBeVisible();
		await openaiRow.getByRole("button", { name: "Choose Model", exact: true }).click();
		await expect(openaiRow.getByText("Select preferred models", { exact: true })).toBeVisible({ timeout: 45_000 });
		const firstModelCard = openaiRow.locator(".model-card").first();
		await expect(firstModelCard).toBeVisible({ timeout: 45_000 });
		const selectedModelId = (await firstModelCard.locator(".font-mono").first().textContent())?.trim() || "";
		expect(selectedModelId).not.toBe("");

		await firstModelCard.click();
		await expect(firstModelCard).toHaveClass(/selected/);

		await page.getByRole("button", { name: "Continue", exact: true }).click();
		await expect(page.getByRole("heading", { name: LLM_STEP_HEADING })).not.toBeVisible({ timeout: 45_000 });

		await page.getByRole("button", { name: "Back", exact: true }).first().click();
		await expect(page.getByRole("heading", { name: LLM_STEP_HEADING })).toBeVisible();

		const openaiRowAfterContinue = page
			.locator(".onboarding-card .rounded-md.border")
			.filter({ has: page.getByText("OpenAI", { exact: true }) })
			.filter({ has: page.getByText("API Key", { exact: true }) })
			.first();

		await expect(openaiRowAfterContinue).toBeVisible();
		await openaiRowAfterContinue.getByRole("button", { name: "Choose Model", exact: true }).click();
		const persistedCard = openaiRowAfterContinue.locator(".model-card").filter({ hasText: selectedModelId }).first();
		await expect(persistedCard).toBeVisible({ timeout: 45_000 });
		await expect(persistedCard).toHaveClass(/selected/);

		expect(pageErrors).toEqual([]);
	});
});
