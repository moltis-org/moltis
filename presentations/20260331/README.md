# 2026-03-31 Moltis Meetup Deck

This folder contains a single-file [Slidev](https://sli.dev) deck for the OpenClaw Meetup Lisbon talk.

## Files

- `slides.md`: main presentation deck

## Run Locally

According to the current Slidev CLI docs, the markdown entry file can be passed directly to `slidev`.

1. Install the CLI if you do not already have it:

```bash
pnpm i -g @slidev/cli
```

Or:

```bash
npm i -g @slidev/cli
```

2. Start the deck:

```bash
slidev presentations/20260331/slides.md --open
```

3. Presenter mode is available from the Slidev UI once the deck is running.

## Export

PDF export:

```bash
slidev export presentations/20260331/slides.md --format pdf
```

Build a static SPA:

```bash
slidev build presentations/20260331/slides.md
```

## Notes

- Speaker notes are embedded in `slides.md` using Slidev comment notes.
- The deck is designed to work without a live demo.
- It references screenshots already present in `website/screenshots/`.
- If you need to trim to 7 minutes, skip the appendix and compress the screenshot slide.
