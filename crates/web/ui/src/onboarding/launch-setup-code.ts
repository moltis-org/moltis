// ── Setup code handed over by the CLI ────────────────────────
//
// When `moltis` opens the browser on first run it appends the setup code
// as `#setup=<code>`. The fragment never reaches the server; we read it once
// at boot and strip it from the address bar so it is not bookmarked or shared.

const SETUP_FRAGMENT = /^#setup=([A-Za-z0-9]{4,32})$/;

let launchSetupCode: string | null = null;

export function captureLaunchSetupCode(): void {
	const match = SETUP_FRAGMENT.exec(window.location.hash);
	if (!match) return;
	launchSetupCode = match[1].toUpperCase();
	window.history.replaceState(window.history.state, "", window.location.pathname + window.location.search);
}

export function getLaunchSetupCode(): string | null {
	return launchSetupCode;
}
