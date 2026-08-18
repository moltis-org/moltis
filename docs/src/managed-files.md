# Managed Files

Managed Files are a persistent, user-managed file tree that Moltis can expose
to agents. Use it for documents and other files that should be available across
sessions without placing them in long-term memory or a sandbox home directory.

## Privacy

```admonish danger title="Managed Files are shared with agent sessions"
Managed Files are not private to the web session that uploaded them. The
product policy is that all agent sessions, including sessions originating from
channels, can read these files whenever their available tools and sandbox mount
permit it. There is no per-session or per-channel Files namespace.

Do not store health records, private data, or other sensitive material here
unless every agent session and channel operator with tool access may read it.
Use tool and channel policy, and set the sandbox mount to `none` when access is
not appropriate.
```

## Files Settings

Open **Settings > Files** to manage the tree. The file browser supports:

- uploading multiple files or a complete folder
- dropping files and nested directory trees into the current folder while
  preserving their relative paths
- browsing folders and listing entries, with folders shown before files
- sorting by filename, modification date, or size in either direction
- creating folders
- downloading files
- renaming files and folders
- moving files and folders to another folder
- deleting files and empty folders, with a separate confirmation before
  recursively deleting a non-empty folder

Writes do not overwrite existing files implicitly. An upload, rename, or move
that conflicts with an existing file requires an explicit overwrite
confirmation. Directories cannot be replaced through the overwrite flow.

Uploads are limited to **1 GiB per file**. Symbolic links, sockets, devices,
FIFOs, and other special files are unsupported; entries must be regular files
or directories. Browser downloads are always returned as attachments rather
than rendered inline. Chromium-based browsers can stream downloads directly to
a selected file. Other browsers buffer downloads up to 64 MiB and reject larger
downloads rather than risk exhausting browser memory.

All Files API and UI operations pass through the normal gateway authentication
gate. API keys need `operator.read` for listing and downloads,
`operator.write` for uploads and mutations, or `operator.admin` for either.

## Paths

The canonical host location is:

```text
<data_dir>/files
```

With the default data directory this is `~/.moltis/files`. If
`MOLTIS_DATA_DIR` or `--data-dir` changes the data directory, the Files root
moves with it.

Moltis advertises the resolved path to agents and supplies
`MOLTIS_FILES_DIR` to local command-based agents and `exec`. Code should use
that variable rather than assuming `~/.moltis/files`:

```sh
ls "$MOLTIS_FILES_DIR"
```

Inside a supported local container sandbox, the canonical path and injected
value are:

```text
/home/sandbox/files
```

There is no dedicated `moltis files` CLI command. For local CLI and shell
workflows, use the resolved host path or `MOLTIS_FILES_DIR` when Moltis has
provided it.

## Sandbox Access

Configure the independent Files mount under `[tools.exec.sandbox]`:

```toml
[tools.exec.sandbox]
managed_files_mount = "ro" # "none", "ro" (default), or "rw"
```

| Value | Agent access inside the sandbox |
|-------|---------------------------------|
| `none` | Managed Files are unavailable. |
| `ro` | Files can be listed and read, but not changed. This is the default. |
| `rw` | Files can be read, created, changed, moved, and deleted. |

The mount is implemented initially for Docker, Podman, and Apple Container.
WASM and remote sandbox backends do not yet expose Managed Files. When `exec`
runs directly on the host, it uses the host Files path; the sandbox mount mode
does not turn host access into a container mount. Normal tool policy, channel
policy, approvals, and host filesystem permissions still apply.

`managed_files_mount` is separate from both `workspace_mount` and
`home_persistence`. In particular, `/home/sandbox/files` is an independent
mount even though its path is below `/home/sandbox`.

## What Files Are Not

Managed Files are deliberately separate from these other surfaces:

- **Session attachments** are uploaded with a particular chat message and are
  stored with session media. Moving a document into Managed Files does not
  attach it to a message, and attaching a document does not add it to Files.
- **Long-term memory and indexing** use `MEMORY.md`, `memory/*.md`, and the
  configured memory backend. Managed Files are not automatically ingested,
  embedded, indexed, searched, summarized, or inserted into model prompts.
- **Shared sandbox home** is controlled by `home_persistence = "shared"` and is
  intended for command-side state such as caches and CLI authentication.
  Managed Files have their own stable root and mount policy.

The system prompt advertises where Managed Files are and whether a supported
sandbox sees them as unavailable, read-only, or read-write. It does **not**
include file names or contents. An agent must choose to list or read a file
through an available tool before its contents enter the conversation context.

## Portable Exports

Managed Files are excluded from portable data exports by default. They may be
included only as an explicit export opt-in. Because that can add arbitrary and
sensitive user files to the archive, review the selection and protect the
resulting backup before moving or sharing it.

Portable imports are bounded to protect the host from compressed archive bombs:
the compressed archive may be at most 2 GiB, its declared uncompressed contents
at most 16 GiB, and the Managed Files subtree at most 8 GiB across 100,000
entries with a 1 GiB limit per file. The imported Files entries and byte total
must exactly match the archive manifest before any changes are applied. Path
metadata is limited to 16 MiB, and the complete archive to 100,000 entries.

Do not confuse this with the separate option for session media: session
attachments and Managed Files are different archive surfaces.

## Security Notes

- The Files service accepts relative logical paths only and rejects traversal
  outside the managed root.
- The service does not follow symbolic links, including links used as parent
  directories.
- Uploads are staged and committed without silently replacing an existing
  destination.
- Read-only sandbox mounts limit sandbox writes, but they do not create
  per-session confidentiality. Every session with permitted read tools can
  still read the same managed tree.

See [Sandbox Backends](sandbox.md), [Filesystem Tools](tools-fs.md), and
[Security Architecture](security.md) for the other policy layers that control
agent access.
