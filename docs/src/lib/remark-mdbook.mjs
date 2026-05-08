function visit(node, visitor) {
  visitor(node)

  if (!node || !Array.isArray(node.children)) {
    return
  }

  for (const child of node.children) {
    visit(child, visitor)
  }
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
}

function parseAdmonition(meta) {
  const value = meta || "note"
  const kind = value.split(/\s+/)[0] || "note"
  const titleMatch = value.match(/title=["']([^"']+)["']/)
  const fallbackTitle = kind.replace(/^./, (char) => char.toUpperCase())

  return {
    kind,
    title: titleMatch ? titleMatch[1] : fallbackTitle,
  }
}

function renderAdmonition(node) {
  const { kind, title } = parseAdmonition(node.meta)
  const paragraphs = node.value
    .split(/\n{2,}/)
    .map((paragraph) => paragraph.trim())
    .filter(Boolean)
    .map((paragraph) => `<p>${escapeHtml(paragraph).replaceAll("\n", "<br>")}</p>`)
    .join("\n")

  return `<aside class="admonition admonition-${escapeHtml(kind)}"><div class="admonition-title">${escapeHtml(title)}</div><div class="admonition-body">${paragraphs}</div></aside>`
}

function rewriteMarkdownHref(href) {
  if (!href || href.startsWith("http://") || href.startsWith("https://") || href.startsWith("#")) {
    return href
  }

  const [rawPath, fragment] = href.split("#")
  const path = rawPath.replace(/^\.\//, "").replace(/^(\.\.\/)+/, "")
  if (!path.endsWith(".md")) {
    return href
  }

  const rewritten = path === "index.md" ? "/index.html" : `/${path.replace(/\.md$/, ".html")}`
  return fragment ? `${rewritten}#${fragment}` : rewritten
}

export function mdbookCompat() {
  return (tree) => {
    visit(tree, (node) => {
      if (node.type === "link") {
        node.url = rewriteMarkdownHref(node.url)
      }

      if (node.type === "code" && node.lang === "admonish") {
        node.type = "html"
        node.value = renderAdmonition(node)
        delete node.lang
        delete node.meta
      }
    })
  }
}
