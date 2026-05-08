import { readFile } from "node:fs/promises"
import { join } from "node:path"

export interface DocPage {
  title: string
  slug: string
  sourcePath: string
  url: string
  section: string
  depth: number
}

export interface DocSection {
  title: string
  pages: DocPage[]
}

const DOCS_ROOT = join(process.cwd(), "src")
const SUMMARY_PATH = join(DOCS_ROOT, "SUMMARY.md")

let cachedIndex: Promise<DocSection[]> | undefined

export async function getDocsIndex(): Promise<DocSection[]> {
  cachedIndex ??= loadDocsIndex()
  return cachedIndex
}

export async function getAllPages(): Promise<DocPage[]> {
  const sections = await getDocsIndex()
  return sections.flatMap((section) => section.pages)
}

export async function getPageBySlug(slug: string): Promise<DocPage | undefined> {
  const pages = await getAllPages()
  return pages.find((page) => page.slug === slug)
}

export async function getPrevNext(slug: string): Promise<{ previous?: DocPage; next?: DocPage }> {
  const pages = await getAllPages()
  const index = pages.findIndex((page) => page.slug === slug)

  if (index === -1) {
    return {}
  }

  return {
    previous: index > 0 ? pages[index - 1] : undefined,
    next: index < pages.length - 1 ? pages[index + 1] : undefined,
  }
}

async function loadDocsIndex(): Promise<DocSection[]> {
  const summary = await readFile(SUMMARY_PATH, "utf8")
  const sections: DocSection[] = []
  let current: DocSection = { title: "Start", pages: [] }

  for (const line of summary.split("\n")) {
    const sectionMatch = line.match(/^#\s+(.+)$/)
    if (sectionMatch && sectionMatch[1] !== "Summary") {
      current = { title: sectionMatch[1], pages: [] }
      sections.push(current)
      continue
    }

    const pageMatch = line.match(/^(\s*)-?\s*\[([^\]]+)\]\(([^)]+\.md)\)/)
    if (!pageMatch) {
      continue
    }

    if (sections.length === 0) {
      sections.push(current)
    }

    const sourcePath = pageMatch[3]
    const slug = sourcePath.replace(/\.md$/, "")
    const normalizedSlug = slug === "index" ? "index" : slug
    current.pages.push({
      title: pageMatch[2],
      slug: normalizedSlug,
      sourcePath,
      url: normalizedSlug === "index" ? "/index.html" : `/${normalizedSlug}.html`,
      section: current.title,
      depth: Math.floor(pageMatch[1].length / 2),
    })
  }

  return sections.filter((section) => section.pages.length > 0)
}
