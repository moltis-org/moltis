// ── GitHub stats badges ─────────────────────────────────────
//
// Fetches open issue and discussion counts from the GitHub REST API
// (unauthenticated, public repo) and caches them in localStorage
// for one hour. Badges are rendered next to the header links.

const REPO = "moltis-org/moltis";
const CACHE_KEY = "moltis-github-stats";
const CACHE_TTL_MS = 60 * 60 * 1000; // 1 hour

interface GitHubStats {
	issues: number | null;
	discussions: number | null;
	fetchedAt: number;
}

function readCache(): GitHubStats | null {
	try {
		const raw = localStorage.getItem(CACHE_KEY);
		if (!raw) return null;
		const cached = JSON.parse(raw) as GitHubStats;
		if (Date.now() - cached.fetchedAt < CACHE_TTL_MS) return cached;
	} catch {
		// Corrupted cache — ignore.
	}
	return null;
}

function writeCache(stats: GitHubStats): void {
	try {
		localStorage.setItem(CACHE_KEY, JSON.stringify(stats));
	} catch {
		// Storage full or unavailable — ignore.
	}
}

function setBadge(id: string, count: number | null): void {
	const el = document.getElementById(id);
	if (!el) return;
	el.textContent = count !== null && count > 0 ? String(count) : "";
}

function applyStats(stats: GitHubStats): void {
	setBadge("githubIssuesCount", stats.issues);
	setBadge("githubDiscussionsCount", stats.discussions);
}

async function fetchIssuesCount(): Promise<number | null> {
	// Use the Search API with type:issue to exclude PRs from the count.
	// Rate limit is 10 req/min unauthenticated, but with 1-hour caching
	// we only make 1 req/hour/user so this is never an issue in practice.
	try {
		const resp = await fetch(
			`https://api.github.com/search/issues?q=repo:${REPO}+type:issue+state:open&per_page=1`,
		);
		if (!resp.ok) return null;
		const data = (await resp.json()) as { total_count?: number };
		return data.total_count ?? null;
	} catch {
		return null;
	}
}

async function fetchDiscussionsCount(): Promise<number | null> {
	// The discussions list endpoint works for public repos without auth.
	// We request per_page=1 and parse the Link header to get the last page number,
	// which equals the total open discussion count.
	try {
		const resp = await fetch(
			`https://api.github.com/repos/${REPO}/discussions?per_page=1`,
		);
		if (!resp.ok) return null;

		const link = resp.headers.get("Link");
		if (link) {
			const match = /[&?]page=(\d+)>;\s*rel="last"/.exec(link);
			if (match) return Number.parseInt(match[1], 10);
		}
		// No Link header means ≤1 page; count the items directly.
		const items = (await resp.json()) as unknown[];
		return items.length;
	} catch {
		return null;
	}
}

async function fetchAndCache(): Promise<void> {
	const [issues, discussions] = await Promise.all([
		fetchIssuesCount(),
		fetchDiscussionsCount(),
	]);
	const stats: GitHubStats = { issues, discussions, fetchedAt: Date.now() };
	writeCache(stats);
	applyStats(stats);
}

// On load: apply cached values immediately, then refresh if stale.
const cached = readCache();
if (cached) {
	applyStats(cached);
} else {
	// No cache — fetch in the background.
	fetchAndCache();
}
