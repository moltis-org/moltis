const { expect, test } = require("@playwright/test");
const { watchPageErrors } = require("../helpers");

const LLM_STEP_HEADING = /^(Add LLMs|Add providers)$/;

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

	const clicked = await clickFirstVisibleButton(page, { name: /skip/i });
	expect(clicked).toBeTruthy();
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

async function maybeSkipOpenClawImport(page) {
	const importHeading = page.getByRole("heading", { name: "Import from OpenClaw", exact: true });
	if (!(await isVisible(importHeading))) return false;

	// The import step has "Skip for now" (when detected) or "Skip" (when not detected)
	const skipBtn = page.getByRole("button", { name: /^Skip/ }).first();
	if (await isVisible(skipBtn)) {
		await skipBtn.click();
	}
	return true;
}

async function moveToLlmStep(page) {
	await waitForOnboardingStepLoaded(page);

	const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
	if (await isVisible(llmHeading)) return true;

	await maybeSkipAuth(page);
	if (await isVisible(llmHeading)) return true;

	await maybeCompleteIdentity(page);
	if (await isVisible(llmHeading)) return true;

	await maybeSkipOpenClawImport(page);
	if (await isVisible(llmHeading)) return true;

	const backBtn = page.getByRole("button", { name: "Back", exact: true }).first();
	if (await isVisible(backBtn)) {
		await backBtn.click();
	}

	await expect(llmHeading).toBeVisible({ timeout: 10_000 });
	return true;
}

async function moveToVoiceStep(page) {
	const reachedLlm = await moveToLlmStep(page);
	if (!reachedLlm) return false;

	const voiceHeading = page.getByRole("heading", { name: "Voice (optional)", exact: true });
	if (await isVisible(voiceHeading)) return true;

	const skipped = await clickFirstVisibleButton(page, { name: "Skip for now", exact: true });
	if (!skipped) return false;

	// Voice step may not exist in the current onboarding flow — return false
	// gracefully instead of throwing when the heading never appears.
	for (let i = 0; i < 20; i++) {
		if (await isVisible(voiceHeading)) return true;
		await page.waitForTimeout(500);
	}
	return false;
}

async function moveToChannelStep(page) {
	const reachedLlm = await moveToLlmStep(page);
	if (!reachedLlm) return false;

	const channelHeading = page.getByRole("heading", { name: "Connect a Channel", exact: true });
	if (await isVisible(channelHeading)) return true;

	for (let i = 0; i < 4; i++) {
		const skipBtn = page.getByRole("button", { name: "Skip for now", exact: true });
		if (!(await isVisible(skipBtn))) break;
		await skipBtn.click();
		if (await isVisible(channelHeading)) return true;
	}

	return isVisible(channelHeading);
}

async function moveToIdentityStep(page) {
	await waitForOnboardingStepLoaded(page);

	const identityHeading = page.getByRole("heading", {
		name: "Set up your identity",
		exact: true,
	});
	if (await isVisible(identityHeading)) return { reached: true, blockedByAuth: false };

	const authHeading = page.getByRole("heading", {
		name: "Secure your instance",
		exact: true,
	});
	if (await isVisible(authHeading)) {
		const authSkippable = await clickFirstVisibleButton(page, { name: "Skip for now", exact: true });
		if (!authSkippable) return { reached: false, blockedByAuth: true };
	}

	for (let i = 0; i < 6; i++) {
		if (await isVisible(identityHeading)) return { reached: true, blockedByAuth: false };

		if (await clickFirstVisibleButton(page, { name: /skip/i })) continue;
		if (await clickFirstVisibleButton(page, { name: /continue/i })) continue;
		break;
	}

	return { reached: await isVisible(identityHeading), blockedByAuth: false };
}

function horizontalOverflowPx(page) {
	return page.evaluate(() => Math.max(0, document.documentElement.scrollWidth - document.documentElement.clientWidth));
}

function firstVisibleOnboardingInputFontSizePx(page) {
	return page.evaluate(() => {
		const inputs = Array.from(document.querySelectorAll(".onboarding-card .provider-key-input"));
		const input = inputs.find((el) => {
			const rect = el.getBoundingClientRect();
			const style = window.getComputedStyle(el);
			return rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden";
		});
		if (!input) return 0;
		return Number.parseFloat(window.getComputedStyle(input).fontSize || "0");
	});
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

	test("server started footer timestamp is hydrated", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);
		if (/^\/chats\//.test(new URL(page.url()).pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const startedTime = page.locator(".onboarding-card time[data-epoch-ms]").first();
		await expect(startedTime).toBeVisible();
		await expect.poll(async () => ((await startedTime.textContent()) || "").trim(), { timeout: 10_000 }).not.toBe("");

		expect(pageErrors).toEqual([]);
	});

	test("step indicator shows first step", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect(page.locator(".onboarding-step-dot").first()).toHaveClass(/active/);
		const activeStepLabel = (
			await page.locator(".onboarding-step.active .onboarding-step-label").first().textContent()
		)?.trim();
		expect(["Security", "Import", "LLM"]).toContain(activeStepLabel);
	});

	test("step indicator orders Import before LLM when import is available", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const labels = (await page.locator(".onboarding-step-label").allTextContents()).map((v) => v.trim());
		const importIdx = labels.indexOf("Import");
		const llmIdx = labels.indexOf("LLM");
		if (importIdx === -1 || llmIdx === -1) {
			test.skip(true, "OpenClaw import is not available in this onboarding run");
		}

		expect(importIdx).toBeLessThan(llmIdx);
	});

	test("auth step renders actionable controls when shown", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
		const isAuthStepVisible = await authHeading.isVisible().catch(() => false);

		if (!isAuthStepVisible) {
			// When auth is not needed, the wizard may show identity, OpenClaw import, or LLM step
			const anyStepHeading = page.getByRole("heading", {
				name: /^(Add LLMs|Add providers|Set up your identity|Import from OpenClaw)$/,
			});
			await expect(anyStepHeading).toBeVisible();
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

		const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
		const identityStep = await moveToIdentityStep(page);

		if (identityStep.blockedByAuth) {
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

		if (!identityStep.reached) {
			const currentHeading = page.locator(".onboarding-card h2").first();
			await expect(currentHeading).toBeVisible();
			const headingText = (await currentHeading.textContent())?.trim() || "";
			expect(["Add LLMs", "Voice (optional)", "Connect a Channel"]).toContain(headingText);
			const canSkip = await clickFirstVisibleButton(page, { name: /skip/i });
			const canContinue = await clickFirstVisibleButton(page, { name: /continue/i });
			expect(canSkip || canContinue).toBeTruthy();
			return;
		}

		await expect(identityHeading).toBeVisible();
		await expect(page.getByPlaceholder("e.g. Alice")).toBeVisible();
		await expect(page.getByPlaceholder("e.g. Rex")).toBeVisible();
		await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeVisible();
	});

	test("mobile onboarding layout avoids horizontal overflow", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.setViewportSize({ width: 375, height: 812 });
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect(page.locator(".onboarding-card")).toBeVisible();
		await expect.poll(() => horizontalOverflowPx(page), { timeout: 10_000 }).toBeLessThan(2);
		const initialInputFontSize = await firstVisibleOnboardingInputFontSizePx(page);
		if (initialInputFontSize > 0) {
			expect(initialInputFontSize).toBeGreaterThanOrEqual(16);
		}

		const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
		if (await authHeading.isVisible().catch(() => false)) {
			const skipBtn = page.getByRole("button", { name: "Skip for now", exact: true });
			if (await skipBtn.isVisible().catch(() => false)) {
				await skipBtn.click();
			}
		}

		const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
		if (await identityHeading.isVisible().catch(() => false)) {
			await expect(page.getByPlaceholder("e.g. Alice")).toBeVisible();
			await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeVisible();
		}

		await expect.poll(() => horizontalOverflowPx(page), { timeout: 10_000 }).toBeLessThan(2);
		const finalInputFontSize = await firstVisibleOnboardingInputFontSizePx(page);
		if (finalInputFontSize > 0) {
			expect(finalInputFontSize).toBeGreaterThanOrEqual(16);
		}
		expect(pageErrors).toEqual([]);
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

		const reachedLlm = await moveToLlmStep(page);
		expect(reachedLlm).toBeTruthy();
		await expect(page.getByRole("heading", { name: LLM_STEP_HEADING })).toBeVisible();
		await page.getByRole("button", { name: "Skip for now", exact: true }).click();

		const channelHeading = page.getByRole("heading", { name: "Connect a Channel", exact: true });
		for (let i = 0; i < 3; i++) {
			if (await channelHeading.isVisible().catch(() => false)) {
				break;
			}
			const skipBtn = page.getByRole("button", { name: "Skip for now", exact: true });
			await expect(skipBtn).toBeVisible();
			await skipBtn.click();
		}

		await expect(channelHeading).toBeVisible();

		let telegramUserInput = page.locator('input[name="telegram_bot_username"]');
		if (!(await isVisible(telegramUserInput))) {
			const telegramSelectBtn = page.getByRole("button", { name: "Telegram", exact: true });
			if (await isVisible(telegramSelectBtn)) {
				await telegramSelectBtn.click();
			}
		}

		telegramUserInput = page.locator('input[name="telegram_bot_username"]');
		if (!(await isVisible(telegramUserInput))) {
			test.skip(true, "Telegram onboarding option is not available in this run");
			return;
		}

		await expect(telegramUserInput).toHaveAttribute("autocomplete", "off");
		await expect(telegramUserInput).toHaveAttribute("name", "telegram_bot_username");
		const tokenInput = page.locator('input[name="telegram_bot_token"]');
		await expect(tokenInput).toHaveAttribute("type", "password");
		await expect(tokenInput).toHaveAttribute("autocomplete", "new-password");
		await expect(tokenInput).toHaveAttribute("name", "telegram_bot_token");
		expect(pageErrors).toEqual([]);
	});

	test("whatsapp pairing renders SVG QR from channel event", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const reachedChannel = await moveToChannelStep(page);
		if (!reachedChannel) {
			test.skip(true, "could not reach channel step in this onboarding flow");
			return;
		}

		const channelHeading = page.getByRole("heading", { name: "Connect a Channel", exact: true });
		await expect(channelHeading).toBeVisible();

		const whatsappSelectBtn = page.getByRole("button", { name: "WhatsApp", exact: true });
		if (await isVisible(whatsappSelectBtn)) {
			await whatsappSelectBtn.click();
		}

		const accountInput = page.getByPlaceholder("e.g. my-whatsapp");
		if (!(await isVisible(accountInput))) {
			test.skip(true, "WhatsApp onboarding option is not available in this run");
			return;
		}

		const accountId = "e2e-whatsapp";

		await page.evaluate(async () => {
			const onboardingScript = document.querySelector('script[type="module"][src*="js/onboarding-app.js"]');
			if (!onboardingScript) throw new Error("onboarding-app.js script not found");
			const appUrl = new URL(onboardingScript.src, window.location.origin).href;
			const marker = "js/onboarding-app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("onboarding-app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.add") {
						resolver({ ok: true, payload: {} });
					} else if (req.method === "channels.status") {
						resolver({ ok: true, payload: { channels: [] } });
					} else {
						resolver({ ok: false, error: { message: `unexpected rpc in onboarding test: ${req.method}` } });
					}
					delete state.pending[req.id];
				},
			});
		});

		await accountInput.fill(accountId);
		await page.getByRole("button", { name: "Start Pairing", exact: true }).click();

		await page.evaluate(
			async ({ accountIdArg }) => {
				const onboardingScript = document.querySelector('script[type="module"][src*="js/onboarding-app.js"]');
				if (!onboardingScript) throw new Error("onboarding-app.js script not found");
				const appUrl = new URL(onboardingScript.src, window.location.origin).href;
				const marker = "js/onboarding-app.js";
				const markerIdx = appUrl.indexOf(marker);
				if (markerIdx < 0) throw new Error("onboarding-app.js marker not found in script URL");
				const prefix = appUrl.slice(0, markerIdx);
				const events = await import(`${prefix}js/events.js`);
				const svg =
					"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 21 21'><rect width='21' height='21' fill='#fff'/><rect x='2' y='2' width='5' height='5' fill='#000'/></svg>";
				const listeners = events.eventListeners.channel || [];
				listeners.forEach((handler) => {
					handler({
						kind: "pairing_qr_code",
						channel_type: "whatsapp",
						account_id: accountIdArg,
						qr_data: "2@mock_payload",
						qr_svg: svg,
					});
				});
			},
			{ accountIdArg: accountId },
		);

		const qrImage = page.locator('img[alt="WhatsApp pairing QR code"]');
		await expect(qrImage).toBeVisible();
		await expect(qrImage).toHaveAttribute("src", /data:image\/svg\+xml;utf8,/);
		await expect(page.getByText("2@mock_payload")).toHaveCount(0);
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

		// Providers with key-source help links. The test picks the first one
		// that shows a "Configure" button (i.e. is not already configured from
		// environment variables). A broad list avoids flakes when the user has
		// several providers pre-configured locally.
		const candidates = [
			{ providerName: "OpenAI", linkName: "OpenAI Platform" },
			{ providerName: "Kimi Code", linkName: "Kimi Code Console" },
			{ providerName: "Anthropic", linkName: "Anthropic Console" },
			{ providerName: "DeepSeek", linkName: "DeepSeek Platform" },
			{ providerName: "Groq", linkName: "Groq Console" },
			{ providerName: "Mistral", linkName: "Mistral Console" },
			{ providerName: "Google Gemini", linkName: "Google AI Studio" },
			{ providerName: "xAI (Grok)", linkName: "xAI Console" },
			{ providerName: "Cerebras", linkName: "Cerebras Cloud" },
			{ providerName: "Venice", linkName: "Venice Settings" },
			{ providerName: "OpenRouter", linkName: "OpenRouter Settings" },
			{ providerName: "Moonshot", linkName: "Moonshot Platform" },
			{ providerName: "MiniMax", linkName: "MiniMax Platform" },
		];
		let matched = false;
		for (const candidate of candidates) {
			const row = page
				.locator(".onboarding-card .rounded-md.border")
				.filter({ has: page.getByText(candidate.providerName, { exact: true }) })
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

		// If every candidate is already configured from env, skip gracefully.
		if (!matched) {
			test.skip(true, "all API-key providers are pre-configured; cannot test key source hint");
			return;
		}
		expect(pageErrors).toEqual([]);
	});

	test("voice needs-key badge uses dedicated pill styling class", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);
		if (/^\/chats\//.test(new URL(page.url()).pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const reachedVoice = await moveToVoiceStep(page);
		if (!reachedVoice) {
			test.skip(true, "voice step not reachable in this onboarding run");
			return;
		}

		const needsKeyBadges = page.locator(".provider-item-badge.needs-key", { hasText: "needs key" });
		const badgeCount = await needsKeyBadges.count();
		if (badgeCount === 0) {
			test.skip(true, "all voice providers already configured");
			return;
		}

		const firstBadge = needsKeyBadges.first();
		await expect(firstBadge).toBeVisible();
		const styles = await firstBadge.evaluate((el) => {
			const computed = window.getComputedStyle(el);
			return {
				background: computed.backgroundColor,
				radius: Number.parseFloat(computed.borderTopLeftRadius || "0"),
			};
		});
		expect(styles.background).not.toBe("transparent");
		expect(styles.background).not.toBe("rgba(0, 0, 0, 0)");
		expect(styles.radius).toBeGreaterThan(8);

		expect(pageErrors).toEqual([]);
	});
});
