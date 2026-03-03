import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, "..");
const sourcePath = path.join(rootDir, "statement.md");
const outputDir = path.join(rootDir, "statement");
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
  value = value.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>');
  value = value.replace(/`([^`]+)`/g, "<code>$1</code>");
  value = value.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  value = value.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  return value;
}

function renderMarkdown(markdown) {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  const html = [];
  let paragraph = [];
  let inList = false;
  let inCode = false;
  let codeLines = [];

  const flushParagraph = () => {
    if (paragraph.length === 0) return;
    const text = paragraph.join(" ").trim();
    if (text) html.push(`<p>${renderInline(text)}</p>`);
    paragraph = [];
  };

  const closeList = () => {
    if (!inList) return;
    html.push("</ul>");
    inList = false;
  };

  const closeCode = () => {
    if (!inCode) return;
    const code = codeLines.map((line) => escapeHtml(line)).join("\n");
    html.push(`<pre><code>${code}</code></pre>`);
    inCode = false;
    codeLines = [];
  };

  for (const line of lines) {
    if (line.startsWith("```")) {
      flushParagraph();
      closeList();
      if (inCode) {
        closeCode();
      } else {
        inCode = true;
      }
      continue;
    }

    if (inCode) {
      codeLines.push(line);
      continue;
    }

    const heading = line.match(/^(#{1,6})\s+(.+)$/);
    if (heading) {
      flushParagraph();
      closeList();
      const level = heading[1].length;
      html.push(`<h${level}>${renderInline(heading[2].trim())}</h${level}>`);
      continue;
    }

    const listItem = line.match(/^\s*-\s+(.+)$/);
    if (listItem) {
      flushParagraph();
      if (!inList) {
        html.push("<ul>");
        inList = true;
      }
      html.push(`<li>${renderInline(listItem[1].trim())}</li>`);
      continue;
    }

    if (line.trim() === "") {
      flushParagraph();
      closeList();
      continue;
    }

    paragraph.push(line.trim());
  }

  flushParagraph();
  closeList();
  closeCode();

  return html.join("\n");
}

function buildHtml(contentHtml) {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta name="color-scheme" content="light dark">
  <title>Moltis Statement</title>
  <meta name="description" content="Why Moltis exists and why it is built in Rust.">
  <link rel="icon" type="image/svg+xml" href="/favicon.svg">
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;500;700&family=JetBrains+Mono:wght@400&display=swap" rel="stylesheet">
  <style>
    :root {
      --bg: #fff8f3;
      --text: #171717;
      --muted: #525252;
      --card: #ffffff;
      --border: #fed7aa;
      --accent: #ea580c;
      --accent-soft: #fff1e8;
      --code-bg: #fff3e0;
    }

    @media (prefers-color-scheme: dark) {
      :root {
        --bg: #111111;
        --text: #fafafa;
        --muted: #d4d4d4;
        --card: #1a1a1a;
        --border: #44403c;
        --accent: #fb923c;
        --accent-soft: #2a1e16;
        --code-bg: #21170f;
      }
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      font-family: "Space Grotesk", system-ui, sans-serif;
      background: radial-gradient(circle at 10% 10%, #fed7aa33 0%, transparent 50%),
                  radial-gradient(circle at 90% 80%, #fb923c22 0%, transparent 45%),
                  var(--bg);
      color: var(--text);
      line-height: 1.65;
      padding: 2rem 1rem 3rem;
    }

    .shell {
      max-width: 860px;
      margin: 0 auto;
      background: var(--card);
      border: 1px solid var(--border);
      border-radius: 16px;
      padding: 2rem 1.5rem;
      box-shadow: 0 8px 40px rgba(0, 0, 0, 0.08);
    }

    .top {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 1rem;
      margin-bottom: 1.25rem;
    }

    .badge {
      display: inline-flex;
      align-items: center;
      padding: 0.3rem 0.65rem;
      border-radius: 999px;
      background: var(--accent-soft);
      color: var(--accent);
      font-size: 0.75rem;
      font-weight: 700;
      letter-spacing: 0.05em;
      text-transform: uppercase;
    }

    .home-link {
      color: var(--accent);
      text-decoration: none;
      font-size: 0.9rem;
      font-weight: 600;
    }

    .home-link:hover { text-decoration: underline; }

    h1, h2, h3 {
      line-height: 1.25;
      margin-top: 1.6em;
      margin-bottom: 0.65em;
    }

    h1 {
      margin-top: 0.2em;
      font-size: clamp(2rem, 4.5vw, 2.8rem);
      letter-spacing: -0.02em;
    }

    h2 {
      font-size: clamp(1.3rem, 3vw, 1.7rem);
      border-top: 1px solid var(--border);
      padding-top: 1rem;
    }

    p { margin: 0.9em 0; color: var(--muted); }

    ul {
      margin: 0.5em 0 1.2em;
      padding-left: 1.25rem;
    }

    li { margin: 0.35em 0; color: var(--muted); }

    code {
      font-family: "JetBrains Mono", monospace;
      background: var(--code-bg);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 0.1em 0.35em;
      font-size: 0.88em;
    }

    pre {
      overflow-x: auto;
      background: var(--code-bg);
      border: 1px solid var(--border);
      border-radius: 10px;
      padding: 0.9rem;
    }

    pre code {
      border: 0;
      background: transparent;
      padding: 0;
    }

    a { color: var(--accent); }
  </style>
</head>
<body>
  <main class="shell">
    <div class="top">
      <span class="badge">Statement</span>
      <a class="home-link" href="/">Back to home</a>
    </div>
    ${contentHtml}
  </main>
</body>
</html>
`;
}

async function main() {
  const markdown = await readFile(sourcePath, "utf8");
  const contentHtml = renderMarkdown(markdown);
  const html = buildHtml(contentHtml);
  await mkdir(outputDir, { recursive: true });
  await writeFile(outputPath, html, "utf8");
  process.stdout.write(`Built statement/index.html from ${path.basename(sourcePath)}\n`);
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exit(1);
});
