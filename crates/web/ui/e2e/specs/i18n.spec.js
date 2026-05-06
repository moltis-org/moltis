const { expect, test } = require("../base-test");
const { expectPageContentMounted, navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

test.describe("i18n", () => {
	test("i18next initialises and English strings render", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.goto("/");
		await expect(page).toHaveURL(/\/chats\/main$/);
		await expectPageContentMounted(page);

		// Verify i18next is initialised on window by checking the module loaded.
		const i18nReady = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) return false;
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			return typeof i18n.t === "function" && typeof i18n.locale?.value === "string";
		});
		expect(i18nReady).toBe(true);

		// Verify the translation function returns English strings (not raw keys).
		const translated = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			return {
				save: i18n.t("common:actions.save"),
				cancel: i18n.t("common:actions.cancel"),
				errorTitle: i18n.t("errors:generic.title"),
				pwaInstallTitle: i18n.t("pwa:install.title"),
				sessionGreeting: i18n.t("sessions:welcome.greetingWithName", { name: "Sam" }),
				locale: i18n.locale.value,
			};
		});
		expect(translated.save).toBe("Save");
		expect(translated.cancel).toBe("Cancel");
		expect(translated.errorTitle).toBe("Error");
		expect(translated.pwaInstallTitle).toBe("Install moltis on your device");
		expect(translated.sessionGreeting).toBe("Hello, Sam!");
		// Default locale should be English (or start with "en").
		expect(translated.locale).toMatch(/^en/);

		expect(pageErrors).toEqual([]);
	});

	test("locale persists to localStorage and survives reload", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.goto("/");
		await expect(page).toHaveURL(/\/chats\/main$/);
		await expectPageContentMounted(page);

		// Set locale to a value and check localStorage.
		const stored = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			// The locale signal should already be set.
			const current = i18n.locale.value;
			// Verify localStorage reflects the detected locale.
			const storedLocale = localStorage.getItem("moltis-locale");
			return { current, storedLocale };
		});
		// On first load without explicit setting, localStorage may or may not be set.
		// After setLocale, it should be persisted.
		expect(stored.current).toMatch(/^en/);

		// Explicitly set locale to "en" and verify persistence.
		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			await i18n.setLocale("en");
		});

		const afterSet = await page.evaluate(() => localStorage.getItem("moltis-locale"));
		expect(afterSet).toBe("en");

		// Reload and verify locale persists.
		await page.reload();
		await expectPageContentMounted(page);

		const afterReload = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			return {
				locale: i18n.locale.value,
				stored: localStorage.getItem("moltis-locale"),
			};
		});
		expect(afterReload.locale).toBe("en");
		expect(afterReload.stored).toBe("en");

		expect(pageErrors).toEqual([]);
	});

	test("unsupported locale falls back to English", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/chats/main");

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			await i18n.setLocale("de-DE");
		});

		await expect.poll(() => page.evaluate(() => document.documentElement.lang)).toBe("en");
		await expect
			.poll(() =>
				page.evaluate(async () => {
					const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
					const appUrl = new URL(appScript.src, window.location.origin);
					const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
					const i18n = await import(`${prefix}js/i18n.js`);
					return {
						locale: i18n.locale.value,
						stored: localStorage.getItem("moltis-locale"),
					};
				}),
			)
			.toEqual({ locale: "en", stored: "en" });

		expect(pageErrors).toEqual([]);
	});

	test("settings page renders translated heading", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/settings/profile");

		// The User Profile heading should render as English text.
		await expect(page.getByRole("heading", { name: "User Profile", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("settings language selector persists and clears locale preference", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/settings/profile");

		const languageSelect = page.locator("#identityLanguageSelect");
		const applyButton = page.locator("#identityLanguageApplyBtn");

		await expect(languageSelect).toBeVisible();
		await expect(applyButton).toBeVisible();

		await languageSelect.selectOption("fr");
		await applyButton.click();
		await expect.poll(() => page.evaluate(() => localStorage.getItem("moltis-locale"))).toBe("fr");
		await waitForWsConnected(page);

		await languageSelect.selectOption("auto");
		await applyButton.click();
		await expect.poll(() => page.evaluate(() => localStorage.getItem("moltis-locale"))).toBe(null);
		await waitForWsConnected(page);

		expect(pageErrors).toEqual([]);
	});

	test("structured error keys resolve to localized chat error text with fallback", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/chats/main");

		const localized = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const helpers = await import(`${prefix}js/helpers.js`);
			return {
				fromKeys: helpers.localizeStructuredError({
					title: "Fallback title",
					detail: "Fallback detail",
					title_key: "errors:chat.rateLimited.title",
					detail_key: "errors:chat.rateLimited.detail",
				}),
				fromFallback: helpers.localizeStructuredError({
					title: "Fallback title",
					detail: "Fallback detail",
					title_key: "errors:missing.title",
					detail_key: "errors:missing.detail",
				}),
			};
		});

		expect(localized.fromKeys.title).toBe("Rate limited");
		expect(localized.fromKeys.detail).toBe("Too many requests. Please wait a moment and try again.");
		expect(localized.fromFallback.title).toBe("Fallback title");
		expect(localized.fromFallback.detail).toBe("Fallback detail");

		expect(pageErrors).toEqual([]);
	});

	test("zh-TW locale loads Traditional Chinese strings", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/chats/main");

		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			await i18n.setLocale("zh-TW");
		});

		const translated = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			return {
				save: i18n.t("common:actions.save"),
				cancel: i18n.t("common:actions.cancel"),
				locale: i18n.locale.value,
			};
		});

		// Verify Traditional Chinese (Taiwan) — not Simplified
		expect(translated.save).toBe("儲存");
		expect(translated.cancel).toBe("取消");
		expect(translated.locale).toBe("zh-TW");

		expect(pageErrors).toEqual([]);
	});

	test("zh-Hant normalizes to zh-TW", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/chats/main");

		const result = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			await i18n.setLocale("zh-Hant");
			return {
				locale: i18n.locale.value,
				translation: i18n.t("common:actions.save"),
			};
		});

		expect(result.locale).toBe("zh-TW");
		expect(result.translation).toBe("儲存");

		expect(pageErrors).toEqual([]);
	});

	test("zh-TW and zh are independent locales", async ({ page }) => {
		const pageErrors = await navigateAndWait(page, "/chats/main");

		// Set to zh (Simplified Chinese) first
		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			await i18n.setLocale("zh");
		});

		const zhResult = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			return { save: i18n.t("common:actions.save"), locale: i18n.locale.value };
		});

		// Simplified Chinese uses 保存, Traditional uses 儲存
		expect(zhResult.save).toBe("保存");
		expect(zhResult.locale).toBe("zh");

		// Now switch to zh-TW
		await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			await i18n.setLocale("zh-TW");
		});

		const twResult = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const i18n = await import(`${prefix}js/i18n.js`);
			return { save: i18n.t("common:actions.save"), locale: i18n.locale.value };
		});

		expect(twResult.save).toBe("儲存");
		expect(twResult.locale).toBe("zh-TW");

		expect(pageErrors).toEqual([]);
	});
});
