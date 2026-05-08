# Frontend Architecture

The moltis web UI is a TypeScript single-page application built with
[Preact](https://preactjs.com/) and [Vite](https://vite.dev/).

## Directory Layout

```
crates/web/
├── ui/                          # TypeScript source & tooling
│   ├── src/                     # Application source
│   │   ├── app.tsx              # Main entry point
│   │   ├── login-app.tsx        # Login page entry
│   │   ├── onboarding-app.tsx   # Onboarding wizard entry
│   │   ├── types/               # Shared type definitions
│   │   ├── stores/              # Preact Signal stores
│   │   ├── components/          # Reusable Preact components
│   │   │   └── forms/           # Form field & layout components
│   │   ├── pages/               # Page components
│   │   │   ├── sections/        # Settings page sections
│   │   │   ├── channels/        # Channel modal sub-components
│   │   │   └── chat/            # Chat page sub-modules
│   │   ├── providers/           # Provider setup sub-modules
│   │   ├── sessions/            # Session management sub-modules
│   │   ├── onboarding/          # Onboarding step components
│   │   ├── ws/                  # WebSocket handler sub-modules
│   │   ├── hooks/               # Custom Preact hooks
│   │   └── locales/             # i18n translations (en, fr, zh)
│   ├── e2e/                     # Playwright E2E tests
│   ├── vite.config.ts           # Vite build configuration
│   ├── tsconfig.json            # TypeScript strict config
│   └── package.json             # Dependencies & scripts
├── src/
│   ├── assets/                  # Served static assets
│   │   ├── dist/                # Vite build output (committed)
│   │   ├── css/                 # Stylesheets (Tailwind + custom)
│   │   ├── js/                  # E2E test shims + share page
│   │   ├── icons/               # Favicons & PWA icons
│   │   └── sw.js                # Service worker
│   └── templates/               # Askama HTML templates
```

## Build Pipeline

### TypeScript → JavaScript (Vite)

Source files in `ui/src/` are compiled and bundled by Vite into
`src/assets/dist/`. Three entry points produce three bundles:

- `dist/main.js` — main app (chat, settings, all pages)
- `dist/login.js` — login page
- `dist/onboarding.js` — onboarding wizard

```bash
cd crates/web/ui
npm run build          # Production build → ../src/assets/dist/
npm run dev            # Watch mode (rebuilds on file changes)
```

The `dist/` output is **committed to git** (unminified, no source maps)
so that `cargo build` works without Node.js installed. This mirrors the
approach used for the committed Tailwind CSS output.

### CSS (Tailwind)

Tailwind CSS is built separately from the TypeScript pipeline:

```bash
cd crates/web/ui
npm run build:css      # input.css → ../src/assets/css/style.css
npm run watch:css      # Watch mode
```

The output `style.css` is committed unminified (one rule per line) so
diffs merge cleanly.

### Service Worker

The service worker is built from TypeScript via esbuild:

```bash
cd crates/web/ui
npm run build:sw       # src/sw.ts → ../src/assets/sw.js
```

### Full Build

```bash
cd crates/web/ui
npm run build:all      # Vite + Tailwind + service worker
```

## Technology Stack

| Layer | Technology |
|-------|-----------|
| UI framework | [Preact](https://preactjs.com/) (lightweight React alternative) |
| Templating | JSX with typed Props interfaces |
| State management | [Preact Signals](https://preactjs.com/guide/v10/signals/) |
| Build tool | [Vite](https://vite.dev/) with `@preact/preset-vite` |
| Type checking | TypeScript strict mode (`tsc --noEmit`) |
| Linting/formatting | [Biome](https://biomejs.dev/) |
| CSS | [Tailwind CSS](https://tailwindcss.com/) v4 |
| i18n | [i18next](https://www.i18next.com/) (en, fr, zh) |
| Charts | [uPlot](https://github.com/leeoniya/uPlot) |
| Terminal | [xterm.js](https://xtermjs.org/) |
| Syntax highlighting | [Shiki](https://shiki.style/) (bundled, lazy-loaded) |
| E2E testing | [Playwright](https://playwright.dev/) |

## Type Safety

The codebase enforces strict TypeScript with zero tolerance for `any`:

- **`tsc --noEmit`** runs in CI and local-validate (must pass with 0 errors)
- **107 typed RPC methods** via `RpcMethodMap` — calling `sendRpc("models.list", {})`
  infers the response type as `ModelInfo[]`
- **28 WebSocket events** via `WsEventName` enum with typed payload discriminated unions
- **`ChannelType` enum** for channel type comparisons (no raw strings)
- **`targetValue(e)` / `targetChecked(e)`** helpers eliminate `(e.target as HTMLInputElement).value` casts

## Shared Component Library

Reusable components in `components/forms/`:

- **Form fields**: `TextField`, `TextAreaField`, `SelectField`, `CheckboxField`
- **Layout**: `SectionHeading`, `SubHeading`, `SettingsCard`, `DangerZone`
- **Lists**: `ListItem`, `Badge`, `EmptyState`, `Loading`, `CopyButton`
- **Navigation**: `TabBar`
- **State**: `useSaveState()` hook, `SaveButton`, `StatusMessage`

## Asset Serving

The Rust `moltis-web` crate serves assets with three-tier resolution:

1. **Dev filesystem** — `MOLTIS_ASSETS_DIR` env var or auto-detected
   from the crate source tree (`cargo run` dev mode)
2. **External share dir** — `share_dir()/web/` for packaged deployments
3. **Embedded fallback** — `include_dir!` compiled into the binary

HTML templates are rendered by [Askama](https://github.com/djc/askama)
with server-injected data (`window.__MOLTIS__`, the "gon" pattern).

## E2E Test Compatibility

E2E tests dynamically import individual JS modules (e.g.,
`await import("js/state.js")`) to inspect and mock internal app state.
With Vite bundling, individual modules don't exist as standalone files.

**Shim layer**: small proxy files in `src/assets/js/` re-export from
`window.__moltis_modules` (populated by `app.tsx` at startup). This
lets tests import modules at their original paths without changes.

The shims are only loaded by E2E tests, never by the production app.

## Development Workflow

After changing TypeScript source files:

```bash
cd crates/web/ui

# 1. Type check
npx tsc --noEmit

# 2. Lint and format
biome check --write src/

# 3. Build (commits dist/ output)
npm run build

# 4. Run E2E tests
npx playwright test --project default
```

For CSS changes, also run `npm run build:css` and commit `style.css`.
