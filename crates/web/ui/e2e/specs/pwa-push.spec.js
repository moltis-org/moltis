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

	test("matches the visible session by whole path, not substring", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();

		// `/chats/main-2` contains `/chats/main`, so a substring test would treat
		// a different chat as on-screen and silence a notification for one the
		// user cannot see.
		expect(source).toContain("new URL(client.url).pathname");
		expect(source).not.toMatch(/client\.url\.includes\(`\/chats\//);
	});

	test("badges the app icon only when it is installed, and never blocks on it", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();

		// The Badging API neither resolves nor rejects where there is no badge
		// target — it hangs. Awaiting it, or gating a waitUntil on it, wedges the
		// worker and the push handler never shows its notification.
		expect(source).toContain("isInstalled()");
		expect(source).not.toMatch(/await\s+nav\.(set|clear)AppBadge/);
		expect(source).not.toMatch(/waitUntil\(\s*updateBadge/);
	});

	// Headed only, by necessity: headless Chromium has no badge target, and the
	// platform call there does not merely fail — it takes the page down. That is
	// exactly what the installed gate exists to avoid, so this test verifies the
	// real installed path where the API actually works. Run with `--headed`.
	test("an installed app badges from the service worker", async ({ page }) => {
		test.skip(test.info().project.use.headless !== false, "Badging API is non-functional in headless Chromium");

		// Emulate standalone display-mode so the page reports itself installed,
		// both now and after the reload.
		await page.addInitScript(() => {
			const original = window.matchMedia.bind(window);
			window.matchMedia = (query) => {
				if (query.includes("display-mode: standalone")) {
					return {
						matches: true,
						media: query,
						onchange: null,
						addEventListener() {},
						removeEventListener() {},
						addListener() {},
						removeListener() {},
						dispatchEvent: () => false,
					};
				}
				return original(query);
			};
		});

		await navigateAndWait(page, "/chats");

		const readFlag = () =>
			page.evaluate(async () => {
				const cache = await caches.open("moltis-state");
				return Boolean(await cache.match("/__moltis__/installed"));
			});

		await expect.poll(readFlag, { timeout: 5000 }).toBe(true);

		// Drive the badge path with the flag on — the platform call runs for real.
		await page.evaluate(async () => {
			const registration = await navigator.serviceWorker.ready;
			registration.active?.postMessage({ type: "CLEAR_NOTIFICATIONS", sessionKey: "main" });
		});

		await page.reload();
		await expect(page.locator("body")).toBeVisible();

		// The flag has to outlive the page, or a push arriving with the app closed
		// would have nothing to tell it the icon is badgeable.
		expect(await readFlag()).toBe(true);
	});

	test("a badge update leaves the service worker responsive", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats");

		// Drive the badge path the same way the app does on focus.
		await page.evaluate(async () => {
			const registration = await navigator.serviceWorker.ready;
			registration.active?.postMessage({ type: "CLEAR_NOTIFICATIONS", sessionKey: "main" });
		});

		// A wedged worker shows up as the next navigation dying, which is exactly
		// how the original badge bug surfaced.
		await page.reload();
		await expect(page.locator("body")).toBeVisible();
		expect(pageErrors).toEqual([]);
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
