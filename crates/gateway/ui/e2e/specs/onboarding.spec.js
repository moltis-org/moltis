const { expect, test } = require("@playwright/test");
const { watchPageErrors } = require("../helpers");

const LLM_STEP_HEADING = /^(Add LLMs|Add providers)$/;

function isVisible(locator) {
	return locator.isVisible().catch(() => false);
}

async function maybeSkipAuth(page) {
	const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
	if (!(await isVisible(authHeading))) return false;

	const authSkip = page.getByRole("button", { name: "Skip for now", exact: true });
	await expect(authSkip).toBeVisible();
	await authSkip.click();
	return true;
}

async function maybeCompleteIdentity(page) {
	const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
	if (!(await isVisible(identityHeading))) return false;

	const userNameInput = page.getByPlaceholder("e.g. Alice");
	if (!(await isVisible(userNameInput))) return false;
	try {
		await userNameInput.fill("E2E User");
	} catch (error) {
		const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
		if (await isVisible(llmHeading)) return false;
		throw error;
	}

	const agentNameInput = page.getByPlaceholder("e.g. Rex");
	if ((await agentNameInput.count()) > 0 && (await isVisible(agentNameInput))) {
		await agentNameInput.fill("E2E Bot");
	}

	await page.getByRole("button", { name: "Continue", exact: true }).click();
	return true;
}

async function moveToLlmStep(page) {
	const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
	if (await isVisible(llmHeading)) return true;

	await maybeSkipAuth(page);
	if (await isVisible(llmHeading)) return true;

	await maybeCompleteIdentity(page);
	if (await isVisible(llmHeading)) return true;

	const backBtn = page.getByRole("button", { name: "Back", exact: true }).first();
	if (await isVisible(backBtn)) {
		await backBtn.click();
	}

	await expect(llmHeading).toBeVisible({ timeout: 10_000 });
	return true;
}

/**
 * Onboarding tests run against a server started WITHOUT seeded
 * IDENTITY.md and USER.md, so the app enters onboarding mode.
 * These use the "onboarding" Playwright project which points at
 * a separate gateway instance on port 18790.
 */
test.describe("Onboarding wizard", () => {
	test.describe.configure({ mode: "serial" });

	test("onboarding gon includes voice_enabled flag", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);

		const pathname = new URL(page.url()).pathname;
		if (/^\/chats\//.test(pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const voiceEnabledType = await page.evaluate(() => typeof window.__MOLTIS__?.voice_enabled);
		expect(voiceEnabledType).toBe("boolean");
		expect(pageErrors).toEqual([]);
	});

	test("redirects to /onboarding on first run", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");

		await expect(page).toHaveURL(/\/onboarding/, { timeout: 15_000 });
		expect(pageErrors).toEqual([]);
	});

	test("step indicator shows first step", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect(page.locator(".onboarding-step-dot").first()).toHaveClass(/active/);
		await expect(page.locator(".onboarding-step-label", { hasText: "Identity" })).toBeVisible();
	});

	test("auth step renders actionable controls when shown", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
		const isAuthStepVisible = await authHeading.isVisible().catch(() => false);

		if (!isAuthStepVisible) {
			await expect(page.getByRole("heading", { name: "Set up your identity", exact: true })).toBeVisible();
			await expect(page.getByPlaceholder("e.g. Alice")).toBeVisible();
			return;
		}

		const passkeyCard = page.locator(".backend-card").filter({ hasText: "Passkey" }).first();
		const passwordCard = page.locator(".backend-card").filter({ hasText: "Password" }).first();
		await expect(passkeyCard).toBeVisible();
		await expect(passwordCard).toBeVisible();

		await passwordCard.click();
		await expect(page.getByRole("button", { name: /Set password|Skip/i }).first()).toBeVisible();
	});

	test("identity step has name input", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const identityHeading = page.getByRole("heading", {
			name: "Set up your identity",
			exact: true,
		});
		const isIdentityStepVisible = await identityHeading.isVisible().catch(() => false);
		if (!isIdentityStepVisible) {
			const skipBtn = page.getByRole("button", { name: /skip/i });
			if (
				await skipBtn
					.first()
					.isVisible()
					.catch(() => false)
			) {
				await skipBtn.first().click();
			} else {
				const authHeading = page.getByRole("heading", {
					name: "Secure your instance",
					exact: true,
				});
				await expect(authHeading).toBeVisible();
				await expect(page.locator(".backend-card").filter({ hasText: "Passkey" }).first()).toBeVisible();
				await expect(page.locator(".backend-card").filter({ hasText: "Password" }).first()).toBeVisible();
				await expect(page.getByText("Setup code", { exact: true })).toBeVisible();
				return;
			}
		}

		await expect(identityHeading).toBeVisible();
		await expect(page.getByPlaceholder("e.g. Alice")).toBeVisible();
		await expect(page.getByPlaceholder("e.g. Rex")).toBeVisible();
		await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeVisible();
	});

	test("page has no JS errors through wizard", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect(page.locator(".onboarding-card")).toBeVisible();
		await expect(page.getByText("Loading…")).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});

	test("telegram bot fields disable credential autofill", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
		if (await authHeading.isVisible().catch(() => false)) {
			const authSkip = page.getByRole("button", { name: "Skip for now", exact: true });
			await expect(authSkip).toBeVisible();
			await authSkip.click();
		}

		const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
		await expect(identityHeading).toBeVisible();
		await page.getByPlaceholder("e.g. Alice").fill("E2E User");
		await page.getByPlaceholder("e.g. Rex").fill("E2E Bot");
		await page.getByRole("button", { name: "Continue", exact: true }).click();

		await expect(page.getByRole("heading", { name: LLM_STEP_HEADING })).toBeVisible();
		await page.getByRole("button", { name: "Skip for now", exact: true }).click();

		const channelHeading = page.getByRole("heading", { name: "Connect Telegram", exact: true });
		for (let i = 0; i < 3; i++) {
			if (await channelHeading.isVisible().catch(() => false)) {
				break;
			}
			const skipBtn = page.getByRole("button", { name: "Skip for now", exact: true });
			await expect(skipBtn).toBeVisible();
			await skipBtn.click();
		}

		await expect(channelHeading).toBeVisible();
		await expect(page.getByPlaceholder("e.g. my_assistant_bot")).toHaveAttribute("autocomplete", "off");
		await expect(page.getByPlaceholder("e.g. my_assistant_bot")).toHaveAttribute("name", "telegram_bot_username");
		const tokenInput = page.getByPlaceholder("123456:ABC-DEF...");
		await expect(tokenInput).toHaveAttribute("type", "password");
		await expect(tokenInput).toHaveAttribute("autocomplete", "new-password");
		await expect(tokenInput).toHaveAttribute("name", "telegram_bot_token");
		expect(pageErrors).toEqual([]);
	});

	test("llm provider api key form includes key source hint", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);

		const pathname = new URL(page.url()).pathname;
		if (/^\/chats\//.test(pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const reachedLlm = await moveToLlmStep(page);
		expect(reachedLlm).toBeTruthy();

		const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
		await expect(llmHeading).toBeVisible();

		const candidates = [
			{ providerName: "OpenAI", linkName: "OpenAI Platform" },
			{ providerName: "Kimi Code", linkName: "Kimi Code Console" },
			{ providerName: "Anthropic", linkName: "Anthropic Console" },
		];
		let matched = false;
		for (const candidate of candidates) {
			const row = page
				.locator(".onboarding-card .rounded-md.border")
				.filter({ hasText: candidate.providerName })
				.first();
			if ((await row.count()) === 0) continue;

			const configureBtn = row.getByRole("button", { name: "Configure", exact: true }).first();
			if (await configureBtn.isVisible().catch(() => false)) {
				await configureBtn.click();
				await expect(page.getByRole("link", { name: candidate.linkName })).toBeVisible();
				matched = true;
				break;
			}
		}

		expect(matched).toBeTruthy();
		expect(pageErrors).toEqual([]);
	});
});
