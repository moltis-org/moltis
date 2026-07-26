const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

// These tests exercise the PWA surface that is observable without a real push
// service: the manifest contract, the offline fallback document, and the
// service worker's notification/caching logic evaluated directly in the page.

test.describe("PWA manifest", () => {
	test("manifest declares the fields installability depends on", async ({ page }) => {
		const response = await page.request.get("/manifest.json");
		expect(response.ok()).toBeTruthy();

		const manifest = await response.json();
		expect(manifest.id).toBeTruthy();
		expect(manifest.name).toBeTruthy();
		expect(manifest.start_url).toBeTruthy();
		expect(manifest.display).toBe("standalone");
		// A locked orientation makes the installed app unusable on tablets and
		// desktop, where the window is landscape by definition.
		expect(manifest.orientation).toBe("any");
		expect(manifest.launch_handler?.client_mode).toBe("navigate-existing");
	});

	test("manifest ships both maskable and any-purpose icons", async ({ page }) => {
		const manifest = await (await page.request.get("/manifest.json")).json();

		const maskable = manifest.icons.filter((icon) => icon.purpose === "maskable");
		expect(maskable.length).toBeGreaterThan(0);

		// Android requires a 192px and a 512px icon for the install prompt.
		const sizes = manifest.icons.map((icon) => icon.sizes);
		expect(sizes).toContain("192x192");
		expect(sizes).toContain("512x512");
	});

	test("every icon referenced by the manifest resolves", async ({ page }) => {
		const manifest = await (await page.request.get("/manifest.json")).json();
		const sources = [...new Set(manifest.icons.map((icon) => icon.src))];

		for (const src of sources) {
			const response = await page.request.get(src);
			expect(response.status(), `icon ${src} must exist`).toBe(200);
		}
	});

	test("every shortcut points at a route the SPA serves", async ({ page }) => {
		const manifest = await (await page.request.get("/manifest.json")).json();

		for (const shortcut of manifest.shortcuts || []) {
			const response = await page.request.get(shortcut.url);
			expect(response.status(), `shortcut ${shortcut.url} must resolve`).toBeLessThan(400);
		}
	});
});

test.describe("PWA offline fallback", () => {
	test("offline page is served and self-contained", async ({ page }) => {
		const response = await page.request.get("/offline.html");
		expect(response.ok()).toBeTruthy();

		const html = await response.text();
		expect(html).toContain("You're offline");
		// The offline page must not depend on any bundle, or it cannot render
		// in the exact situation it exists for.
		expect(html).not.toMatch(/<script[^>]+src=/);
	});
});

test.describe("service worker", () => {
	test("registers and controls the page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats");

		const scope = await page.evaluate(async () => {
			const registration = await navigator.serviceWorker.ready;
			return registration.scope;
		});

		expect(scope).toContain("/");
		expect(pageErrors).toEqual([]);
	});

	test("does not activate a waiting worker without the page asking", async ({ page }) => {
		await navigateAndWait(page, "/chats");

		// The worker script must not call skipWaiting() at install time, or an
		// update reloads the app mid-conversation.
		const source = await (await page.request.get("/sw.js")).text();
		const installBlock = source.slice(source.indexOf('addEventListener("install"'));
		const activateIndex = installBlock.indexOf('addEventListener("activate"');
		const installBody = activateIndex === -1 ? installBlock : installBlock.slice(0, activateIndex);
		expect(installBody).not.toContain("skipWaiting");

		// It must still honour an explicit request from the page.
		expect(source).toContain("SKIP_WAITING");
	});

	test("precaches assets individually so one missing file cannot fail install", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();
		// cache.addAll() is atomic: a single 404 rejects the whole install and
		// the worker never activates.
		expect(source).not.toContain("addAll");
		expect(source).toContain("allSettled");
	});

	test("handles push subscription rotation", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();
		// Without this the endpoint silently rotates and push stops working
		// until the user toggles it off and on again.
		expect(source).toContain("pushsubscriptionchange");
	});

	test("re-alerts instead of silently replacing a same-session notification", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();
		expect(source).toContain("renotify");
	});
});

test.describe("push settings", () => {
	test("notifications section renders its current state", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/notifications");

		await expect(page.getByText("Notifications", { exact: true }).first()).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("booting the app does not query pushManager without permission", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats");

		// Regression guard: initPWA runs presence reporting on every page load.
		// Reaching for pushManager.getSubscription() there — with no granted
		// permission and no push backend — crashes the renderer, taking down the
		// whole app rather than just push. The permission check must short-circuit.
		const state = await page.evaluate(() => ({
			permission: typeof Notification === "undefined" ? "unsupported" : Notification.permission,
			alive: document.readyState,
		}));

		expect(state.permission).not.toBe("granted");
		expect(state.alive).toBe("complete");
		expect(pageErrors).toEqual([]);
	});

	test("presence endpoint rejects an endpoint the server does not know", async ({ page }) => {
		const response = await page.request.post("/api/push/presence", {
			data: {
				endpoint: "https://push.example.com/definitely-not-registered",
				session_key: "main",
				visible: true,
			},
		});

		// 404 tells the client its subscription is stale; 501 means the server
		// was built without the push-notifications feature.
		expect([404, 501]).toContain(response.status());
	});
});
