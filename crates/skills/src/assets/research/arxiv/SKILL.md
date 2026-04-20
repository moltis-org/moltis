---
name: arxiv
description: Search and retrieve academic papers from arXiv using their free REST API. No API key needed. Search by keyword, author, category, or ID.
origin:
  source: hermes-agent
  url: https://github.com/nousresearch/hermes-agent
  version: 9f22977f
allowed-tools:
  - exec
  - web_fetch
---

# arXiv Paper Search

Search and retrieve academic papers from arXiv's free REST API. No API key required.

## Quick Reference

| Action | Method |
|--------|--------|
| Search papers | GET `http://export.arxiv.org/api/query?search_query=...` |
| Get by ID | GET `http://export.arxiv.org/api/query?id_list=2301.07041` |
| Read abstract | Parse `<summary>` from Atom XML response |
| Read PDF | Fetch `https://arxiv.org/pdf/{id}` |

## Search API

Base URL: `http://export.arxiv.org/api/query`

### Query syntax

| Prefix | Searches | Example |
|--------|----------|---------|
| `all:` | All fields | `all:transformer` |
| `ti:` | Title | `ti:attention+mechanism` |
| `au:` | Author | `au:vaswani` |
| `abs:` | Abstract | `abs:large+language+model` |
| `cat:` | Category | `cat:cs.AI` |

### Boolean operators

- `AND` — both terms: `ti:attention AND au:vaswani`
- `OR` — either term: `cat:cs.CL OR cat:cs.AI`
- `ANDNOT` — exclude: `ti:transformer ANDNOT ti:vision`
- Exact phrase: `ti:"chain of thought"`

### Pagination

- `start=0` — offset (default 0)
- `max_results=10` — results per page (default 10, max 100)
- `sortBy=submittedDate` — sort field (`relevance`, `lastUpdatedDate`, `submittedDate`)
- `sortOrder=descending` — sort direction

### Example: search for recent LLM papers

```bash
curl -s 'http://export.arxiv.org/api/query?search_query=ti:large+language+model&sortBy=submittedDate&sortOrder=descending&max_results=5'
```

The response is Atom XML. Parse `<entry>` elements for:
- `<id>` — arXiv URL (extract ID from path)
- `<title>` — paper title
- `<summary>` — abstract
- `<author><name>` — author names
- `<published>` — publication date
- `<link href="..." title="pdf"/>` — PDF link

### Fetch a specific paper

```bash
curl -s 'http://export.arxiv.org/api/query?id_list=2301.07041'
```

## Common categories

| Code | Field |
|------|-------|
| `cs.AI` | Artificial Intelligence |
| `cs.CL` | Computation and Language (NLP) |
| `cs.CV` | Computer Vision |
| `cs.LG` | Machine Learning |
| `cs.CR` | Cryptography and Security |
| `cs.SE` | Software Engineering |
| `stat.ML` | Machine Learning (Statistics) |
| `math.OC` | Optimization and Control |

## Research Workflow

1. **Discover** — search by keyword/category for recent papers
2. **Assess** — scan titles and abstracts from search results
3. **Read abstract** — parse `<summary>` for promising papers
4. **Read full paper** — fetch PDF via `https://arxiv.org/pdf/{id}`
5. **Find related** — search by shared authors or cited paper IDs
6. **Track authors** — search `au:lastname` for an author's full publication list

## Rate Limits

- arXiv API is free but rate-limited. Add 3-second delays between requests.
- Do not make more than 1 request per 3 seconds.
- Bulk downloads should use the arXiv bulk data access instead.
