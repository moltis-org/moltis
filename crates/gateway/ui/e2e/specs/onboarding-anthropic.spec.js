const { expect, test } = require("@playwright/test");
const { watchPageErrors } = require("../helpers");

const LLM_STEP_HEADING = /^(Add LLMs|Add providers)$/;
const ANTHROPIC_API_KEY = process.env.MOLTIS_E2E_ANTHROPIC_API_KEY || process.env.ANTHROPIC_API_KEY || "";

function isVisible(locator) {
	return locator.isVisible().catch(() => false);
}

async function maybeSkipAuth(page) {
	const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
	if (!(await isVisible(authHeading))) return;

	const authSkip = page.getByRole("button", { name: "Skip for now", exact: true });
	await expect(authSkip).toBeVisible();
	await authSkip.click();
}

async function maybeCompleteIdentity(page) {
	const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
	if (!(await isVisible(identityHeading))) return;

	await page.getByPlaceholder("e.g. Alice").fill("E2E User");
	const agentInput = page.getByPlaceholder("e.g. Rex");
	if (await isVisible(agentInput)) {
		await agentInput.fill("E2E Bot");
	}
	await page.getByRole("button", { name: "Continue", exact: true }).click();
}

async function moveToLlmStep(page) {
	const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
	if (await isVisible(llmHeading)) return;

	await maybeSkipAuth(page);
	if (await isVisible(llmHeading)) return;

	await maybeCompleteIdentity(page);
	if (await isVisible(llmHeading)) return;

	await expect(llmHeading).toBeVisible({ timeout: 15_000 });
}

async function selectModelAndSave(anthropicRow) {
	const modelCards = anthropicRow.locator(".model-card");
	await expect(modelCards.first()).toBeVisible({ timeout: 45_000 });
	await expect.poll(() => modelCards.count(), { timeout: 45_000 }).toBeGreaterThan(0);
	await modelCards.first().click();
	await expect(modelCards.first()).toHaveClass(/selected/, { timeout: 5_000 });
	await anthropicRow.getByRole("button", { name: "Save", exact: true }).click();
	await expect(anthropicRow.locator(".provider-item-badge.configured")).toBeVisible({ timeout: 45_000 });
}

test.describe("Onboarding Anthropic provider", () => {
	test.describe.configure({ mode: "serial" });

	test.skip(!ANTHROPIC_API_KEY, "requires ANTHROPIC_API_KEY or MOLTIS_E2E_ANTHROPIC_API_KEY");

	test("starts with no detected providers, then configures Anthropic and loads models", async ({ page }) => {
		test.setTimeout(90_000);
		const pageErrors = watchPageErrors(page);

		await page.goto("/onboarding");
		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);

		if (/^\/chats\//.test(new URL(page.url()).pathname)) {
			// Previous partial runs can land in chats when onboarding already completed.
			// This project uses an isolated runtime, so this should be rare.
			expect(pageErrors).toEqual([]);
			return;
		}

		await moveToLlmStep(page);
		await expect(page.getByRole("heading", { name: LLM_STEP_HEADING })).toBeVisible();

		// Validate startup had no pre-detected LLM provider badges.
		await expect(page.getByText("Detected LLM providers", { exact: true })).toHaveCount(0);

		const anthropicRow = page
			.locator(".onboarding-card .rounded-md.border")
			.filter({ has: page.getByText("Anthropic", { exact: true }) })
			.filter({ has: page.getByText("API Key", { exact: true }) })
			.first();
		await expect(anthropicRow).toBeVisible();
		await expect(anthropicRow.getByRole("button", { name: "Configure", exact: true })).toBeVisible();
		await expect(anthropicRow.locator(".provider-item-badge.configured")).toHaveCount(0);

		await anthropicRow.getByRole("button", { name: "Configure", exact: true }).click();
		await anthropicRow.locator("input[type='password']").first().fill(ANTHROPIC_API_KEY);
		await anthropicRow.getByRole("button", { name: "Save & Validate", exact: true }).click();

		await expect(anthropicRow.getByText("Select preferred models", { exact: true })).toBeVisible({ timeout: 45_000 });
		await selectModelAndSave(anthropicRow);

		// Successful save + model probe collapses the form and marks provider configured.
		await expect(anthropicRow.locator(".provider-item-badge.configured")).toBeVisible({ timeout: 45_000 });
		await expect(anthropicRow.getByRole("button", { name: "Choose Model", exact: true })).toBeVisible();
		await expect(page.getByText("Detected LLM providers", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
