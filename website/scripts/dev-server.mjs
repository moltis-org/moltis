import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { localizeNavHtml, resolvePageLang } from "./nav-i18n.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");
const port = parseInt(process.env.PORT || "4000", 10);

const MIME = {
	".html": "text/html; charset=utf-8",
	".css": "text/css",
	".js": "application/javascript",
	".json": "application/json",
	".svg": "image/svg+xml",
	".png": "image/png",
	".jpg": "image/jpeg",
	".jpeg": "image/jpeg",
	".ico": "image/x-icon",
	".woff2": "font/woff2",
	".txt": "text/plain",
	".sh": "text/plain",
	".xml": "application/xml",
};

let navCache = null;

async function loadNav() {
	if (!navCache) {
		try {
			navCache = await readFile(path.join(root, "_partials", "nav.html"), "utf8");
		} catch {
			navCache = "<!-- nav partial not found -->";
		}
	}
	return navCache;
}

async function tryFile(filePath) {
	try {
		const s = await stat(filePath);
		if (s.isFile()) return filePath;
	} catch {}
	return null;
}

async function resolveFile(pathname) {
	// Exact file
	let file = await tryFile(path.join(root, pathname));
	if (file) return file;

	// Directory → index.html
	file = await tryFile(path.join(root, pathname, "index.html"));
	if (file) return file;

	return null;
}

const server = createServer(async (req, res) => {
	const url = new URL(req.url, `http://localhost:${port}`);
	let pathname = decodeURIComponent(url.pathname);

	// Root → index.en.html
	if (pathname === "/") pathname = "/index.en.html";

	const filePath = await resolveFile(pathname);
	if (!filePath) {
		res.writeHead(404, { "content-type": "text/plain" });
		res.end("404 Not Found");
		return;
	}

	const ext = path.extname(filePath);
	const contentType = MIME[ext] || "application/octet-stream";

	let body = await readFile(filePath);

	// Inject nav partial into HTML
	if (ext === ".html") {
		let html = body.toString("utf8");
		if (html.includes("<!--NAV-->")) {
			const nav = await loadNav();
			html = html.replace("<!--NAV-->", localizeNavHtml(nav, resolvePageLang(html)));
		}
		body = html;
	}

	res.writeHead(200, { "content-type": contentType });
	res.end(body);
});

// Invalidate nav cache on file change (for live editing)
import { watch } from "node:fs";
watch(path.join(root, "_partials"), { recursive: true }, () => {
	navCache = null;
});

server.listen(port, () => {
	process.stdout.write(`Website dev server: http://localhost:${port}\n`);
});
