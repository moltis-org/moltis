# Moltis Docs

The published documentation at <https://docs.moltis.org> is built with Astro from the Markdown files in `docs/src/`.

## Local development

```bash
cd docs
npm ci
npm run dev
```

## Production build

```bash
cd docs
npm run build
npm run preview
```

The GitHub Actions docs workflow builds `docs/dist/` and publishes it to GitHub Pages on pushes to `main`.
