import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, "..");
const projectRoot = path.resolve(rootDir, "..");
const sourcePath = path.join(projectRoot, "CHANGELOG.md");
const outputDir = path.join(rootDir, "changelog");
const outputPath = path.join(outputDir, "index.html");

function escapeHtml(value) {
	return value
		.replaceAll("&", "&amp;")
		.replaceAll("<", "&lt;")
		.replaceAll(">", "&gt;")
		.replaceAll('"', "&quot;")
		.replaceAll("'", "&#39;");
}

function renderInline(raw) {
	let value = escapeHtml(raw);
	value = value.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" class="text-orange-600 dark:text-orange-400 hover:underline">$1</a>');
	value = value.replace(/`([^`]+)`/g, '<code class="px-1 py-0.5 bg-gray-100 dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded text-[0.85em]">$1</code>');
	value = value.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
	value = value.replace(/\*([^*]+)\*/g, "<em>$1</em>");
	// Turn (#123) into PR/issue links
	value = value.replace(/\(#(\d+)\)/g, '(<a href="https://github.com/moltis-org/moltis/pull/$1" target="_blank" rel="noopener" class="text-orange-600 dark:text-orange-400 hover:underline">#$1</a>)');
	return value;
}

/** Turn a version heading like "[20260327.02] - 2026-03-27" into a URL-safe id. */
function versionSlug(text) {
	const match = text.match(/\[([^\]]+)\]/);
	return match ? match[1] : text.toLowerCase().replace(/[^a-z0-9]+/g, "-");
}

function splitSections(markdown) {
	const lines = markdown.replace(/\r\n/g, "\n").split("\n");
	const preamble = [];
	const sections = [];
	let current = null;

	for (const line of lines) {
		const heading = line.match(/^## \[([0-9]{8}\.[0-9]{1,2}|Unreleased)\]/);
		if (heading) {
			current = { version: heading[1], heading: line, lines: [] };
			sections.push(current);
			continue;
		}

		if (current) {
			current.lines.push(line);
		} else {
			preamble.push(line);
		}
	}

	return { preamble, sections };
}

function releaseItems(lines) {
	return new Set(lines.map((line) => line.trim()).filter((line) => line.startsWith("- ")));
}

function removeEmptyCategories(lines, duplicateItems) {
	const output = [];
	let index = 0;

	while (index < lines.length) {
		const line = lines[index];
		if (!line.startsWith("### ")) {
			output.push(line);
			index += 1;
			continue;
		}

		const heading = line;
		const body = [];
		index += 1;
		while (index < lines.length && !lines[index].startsWith("### ")) {
			const bodyLine = lines[index];
			if (!duplicateItems.has(bodyLine.trim())) {
				body.push(bodyLine);
			}
			index += 1;
		}

		if (body.some((bodyLine) => bodyLine.trim() !== "")) {
			output.push(heading, ...body);
		}
	}

	return output;
}

function hasReleaseContent(lines) {
	return lines.some((line) => {
		const text = line.trim();
		return text !== "" && !text.startsWith("### ");
	});
}

function normalizeReleaseDeltas(markdown) {
	const { preamble, sections } = splitSections(markdown);
	if (sections.length === 0) return markdown;

	const normalized = [];
	for (let index = 0; index < sections.length; index += 1) {
		const section = sections[index];
		let lines = section.lines;

		if (section.version !== "Unreleased") {
			const previousRelease = sections.slice(index + 1).find((candidate) => candidate.version !== "Unreleased");
			if (previousRelease) {
				// Compare against the raw older section: cumulative changelogs become deltas
				// by subtracting that raw prior snapshot.
				lines = removeEmptyCategories(section.lines, releaseItems(previousRelease.lines));
			}
			if (!hasReleaseContent(lines)) {
				lines = ["", "No notable changes."];
			}
		}

		normalized.push(section.heading, ...lines);
	}

	return [...preamble, ...normalized].join("\n");
}

const categoryClasses = {
	added: "bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400",
	fixed: "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-400",
	changed: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400",
	removed: "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400",
	security: "bg-violet-100 text-violet-700 dark:bg-violet-900/40 dark:text-violet-400",
	deprecated: "bg-stone-100 text-stone-600 dark:bg-stone-800/40 dark:text-stone-400",
};

function renderMarkdown(markdown) {
	const lines = markdown.replace(/\r\n/g, "\n").split("\n");
	const html = [];
	let paragraph = [];
	let inList = false;

	const flushParagraph = () => {
		if (paragraph.length === 0) return;
		const text = paragraph.join(" ").trim();
		if (text) html.push(`<p class="text-sm text-gray-500 dark:text-gray-400 my-1">${renderInline(text)}</p>`);
		paragraph = [];
	};

	const closeList = () => {
		if (!inList) return;
		html.push("</ul>");
		inList = false;
	};

	for (const line of lines) {
		if (line.match(/^#\s+Changelog/i)) continue;
		if (line.match(/^All notable changes/i)) continue;
		if (line.match(/^and this project adheres/i)) continue;

		const heading = line.match(/^(#{1,6})\s+(.+)$/);
		if (heading) {
			flushParagraph();
			closeList();
			const level = heading[1].length;
			const text = heading[2].trim();

			if (level === 2) {
				const slug = versionSlug(text);
				const display = text.replace(/^\[([^\]]+)\]/, "$1");
				html.push(`<div class="mt-8 first:mt-0 border-t border-gray-200 dark:border-gray-800" id="${escapeHtml(slug)}"></div>`);
				html.push(`<div class="sticky top-14 z-10 bg-white/95 dark:bg-gray-950/95 backdrop-blur-sm py-2 -mx-4 px-4 sm:-mx-6 sm:px-6">`);
				html.push(`<h2 class="font-mono text-lg font-bold text-gray-900 dark:text-white m-0"><a href="#${escapeHtml(slug)}" class="no-underline hover:text-orange-600 dark:hover:text-orange-400">${escapeHtml(display)}</a></h2>`);
				html.push("</div>");
			} else if (level === 3) {
				const category = text.toLowerCase();
				const cls = categoryClasses[category] || "bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400";
				html.push(`<span class="inline-block mt-3 mb-1 px-2 py-0.5 rounded text-[0.7rem] font-bold uppercase tracking-wider ${cls}">${escapeHtml(text)}</span>`);
			}
			continue;
		}

		const listItem = line.match(/^\s*-\s+(.+)$/);
		if (listItem) {
			flushParagraph();
			if (!inList) {
				html.push('<ul class="list-none m-0 pl-0 space-y-0.5">');
				inList = true;
			}
			const raw = listItem[1].trim();
			const scopeMatch = raw.match(/^\[([^\]]+)\]\s*(.*)$/);
			let inner;
			if (scopeMatch) {
				const scope = escapeHtml(scopeMatch[1]);
				const rest = renderInline(scopeMatch[2]);
				inner = `<span class="inline-block w-[7rem] shrink-0 text-right pr-2 text-gray-400 dark:text-gray-500 select-none">[${scope}]</span><span>${rest}</span>`;
			} else {
				inner = `<span class="inline-block w-[7rem] shrink-0"></span><span>${renderInline(raw)}</span>`;
			}
			html.push(`<li class="font-mono text-[0.8rem] leading-snug text-gray-600 dark:text-gray-300 flex">${inner}</li>`);
			continue;
		}

		if (line.trim() === "") {
			flushParagraph();
			closeList();
			// Skip empty sections inside Unreleased
			continue;
		}

		paragraph.push(line.trim());
	}

	flushParagraph();
	closeList();
	return html.join("\n");
}

function buildHtml(contentHtml) {
	return `<!DOCTYPE html>
<html lang="en">
<head>
    <script>
        (function(){
            var t = localStorage.getItem('theme') || 'system';
            if (t === 'dark' || (t === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)) {
                document.documentElement.classList.add('dark');
            }
        })();
    </script>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta name="color-scheme" content="light dark">
    <title>Changelog - Moltis</title>
    <meta name="description" content="Release history and changelog for Moltis.">
    <link rel="icon" type="image/svg+xml" href="/favicon.svg">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;600;700&family=JetBrains+Mono:wght@400&family=Outfit:wght@700&display=swap" rel="stylesheet">
    <script src="https://cdn.tailwindcss.com"></script>
    <script>
        tailwind.config = {
            darkMode: 'class',
            theme: {
                extend: {
                    fontFamily: {
                        sans: ['Space Grotesk', 'system-ui', 'sans-serif'],
                        mono: ['JetBrains Mono', 'monospace'],
                    },
                },
            },
        }
        function applyTheme(mode) {
            if (mode === 'dark' || (mode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)) {
                document.documentElement.classList.add('dark');
            } else {
                document.documentElement.classList.remove('dark');
            }
        }
        const savedTheme = localStorage.getItem('theme') || 'system';
        applyTheme(savedTheme);
        window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
            if ((localStorage.getItem('theme') || 'system') === 'system') applyTheme('system');
        });
    </script>
    <style>
        @keyframes pulse-slow {
            0%, 100% { opacity: 0.3; }
            50% { opacity: 0.7; }
        }
        .animate-pulse-slow { animation: pulse-slow 4s ease-in-out infinite; }
        .animation-delay-2s { animation-delay: 2s; }
        .font-outfit { font-family: 'Outfit', sans-serif; }
    </style>
</head>
<body class="bg-white dark:bg-gray-950 text-gray-900 dark:text-white min-h-screen relative transition-colors duration-300 overflow-x-hidden w-full">
    <!-- Background gradient blobs -->
    <div class="absolute inset-0 overflow-hidden pointer-events-none">
        <div class="absolute top-1/4 left-1/4 w-64 h-64 sm:w-96 sm:h-96 bg-orange-300/20 dark:bg-orange-600/15 rounded-full blur-3xl animate-pulse-slow"></div>
        <div class="absolute bottom-1/4 right-1/4 w-64 h-64 sm:w-96 sm:h-96 bg-red-300/20 dark:bg-red-600/15 rounded-full blur-3xl animate-pulse-slow animation-delay-2s"></div>
    </div>

    <!--NAV-->

    <!-- Content -->
    <main class="relative max-w-4xl mx-auto px-4 sm:px-6 py-8">
        ${contentHtml}
    </main>

    <script>
        // Theme toggle
        const modes = ['system', 'light', 'dark'];
        const icons = { light: 'theme-icon-light', system: 'theme-icon-system', dark: 'theme-icon-dark' };
        let current = localStorage.getItem('theme') || 'system';
        function updateIcon() {
            Object.values(icons).forEach(id => document.getElementById(id).classList.add('hidden'));
            document.getElementById(icons[current]).classList.remove('hidden');
        }
        updateIcon();
        document.getElementById('theme-toggle').addEventListener('click', () => {
            current = modes[(modes.indexOf(current) + 1) % modes.length];
            localStorage.setItem('theme', current);
            applyTheme(current);
            updateIcon();
        });

        // Highlight active nav tab
        document.querySelectorAll('.nav-tab').forEach(function(tab) {
            if (tab.dataset.page === 'changelog') {
                tab.className = tab.className.replace('text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-800', '');
                tab.classList.add('bg-orange-500', 'text-white');
            }
        });

        // GitHub stars are fetched by the shared nav partial script.
    </script>
</body>
</html>
`;
}

async function main() {
	const markdown = await readFile(sourcePath, "utf8");
	const contentHtml = renderMarkdown(normalizeReleaseDeltas(markdown));
	const html = buildHtml(contentHtml);
	await mkdir(outputDir, { recursive: true });
	await writeFile(outputPath, html, "utf8");
	process.stdout.write(`Built changelog/index.html from CHANGELOG.md\n`);
}

main().catch((error) => {
	process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
	process.exit(1);
});
