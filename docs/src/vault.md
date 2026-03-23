# Encryption at Rest (Vault)

Moltis includes an encryption-at-rest vault that protects sensitive data
stored in the SQLite database. Environment variables (provider API keys,
tokens, etc.) are encrypted with **XChaCha20-Poly1305** AEAD using keys
derived from your password via **Argon2id**.

The vault is enabled by default (the `vault` cargo feature) and requires
no configuration. It initializes automatically when you set your first
password (during setup or later in **Settings > Authentication**).

## Key Hierarchy

The vault uses a two-layer key hierarchy to separate the encryption key
from the password:

```
User password
  │
  ▼ Argon2id (salt from DB)
  │
  KEK (Key Encryption Key)
  │
  ▼ XChaCha20-Poly1305 unwrap
  │
  DEK (Data Encryption Key)
  │
  ▼ XChaCha20-Poly1305 encrypt/decrypt
  │
  Encrypted data (env variables, ...)
```

- **KEK** — derived from the user's password using Argon2id with a
  per-instance random salt. Never stored directly; recomputed on each unseal.
- **DEK** — a random 256-bit key generated once at vault initialization.
  Stored encrypted (wrapped) by the KEK in the `vault_metadata` table.
- **Recovery KEK** — an independent Argon2id-derived key from the recovery
  phrase with a fixed domain-separation salt, used to wrap a second copy of
  the DEK for emergency access. Uses lighter KDF parameters (16 MiB, 2
  iterations) since the recovery key already has 128 bits of entropy.

This design means changing your password only re-wraps the DEK with a new
KEK. The DEK itself (and all data encrypted by it) stays the same, so
password changes are fast regardless of how much data is encrypted.

## Vault States

The vault has three states:

| State | Meaning |
|-------|---------|
| **Uninitialized** | No vault metadata exists. The vault hasn't been set up yet. |
| **Sealed** | Metadata exists but the DEK is not in memory. Data cannot be read or written. |
| **Unsealed** | The DEK is in memory. Encryption and decryption are active. |

```
                 set password
Uninitialized ──────────────► Unsealed
                                │  ▲
                     restart    │  │  login / unlock
                                ▼  │
                              Sealed
```

After a server restart, the vault is always in the **Sealed** state until
the user logs in (which provides the password needed to derive the KEK and
unwrap the DEK).

## Lifecycle Integration

The vault integrates transparently with the authentication flow:

### First password set (`POST /api/auth/setup` or first `POST /api/auth/password/change`)

When the first password is set (during onboarding or later in Settings):

1. `vault.initialize(password)` generates a random DEK and recovery key
2. The DEK is wrapped with a KEK derived from the password
3. A second copy of the DEK is wrapped with the recovery KEK
4. The response includes a `recovery_key` field (shown once, then not returned again)
5. Any existing plaintext env vars are migrated to encrypted

### Login (`POST /api/auth/login`)

After successful password verification:

1. `vault.unseal(password)` derives the KEK and unwraps the DEK into memory
2. Unencrypted env vars are migrated to encrypted (if any remain)

### Password change after initialization (`POST /api/auth/password/change`)

When a password already exists and is rotated:

1. `vault.change_password(old, new)` re-wraps the DEK with a new KEK
   derived from the new password

No new recovery key is generated during normal password rotation.

### Server restart

The vault starts in **Sealed** state. All encrypted data is unreadable
until the user logs in, which triggers unseal.

## Recovery Key

At vault initialization, a human-readable recovery key is generated and
returned in the API response that performed initialization. It looks like:

```
ABCD-EFGH-JKLM-NPQR-STUV-WXYZ-2345-6789
```

The alphabet excludes ambiguous characters (`I`, `O`, `0`, `1`) to avoid
transcription errors. The key is case-insensitive.

```admonish warning
The recovery key is shown **exactly once** when the vault is initialized.
Store it in a safe place (password manager, printed copy in a safe, etc.).
If you lose both your password and recovery key, encrypted data cannot be
recovered.
```

Use the recovery key to unseal the vault when you've forgotten your
password:

```bash
curl -X POST http://localhost:18789/api/auth/vault/recovery \
  -H "Content-Type: application/json" \
  -d '{"recovery_key": "ABCD-EFGH-JKLM-NPQR-STUV-WXYZ-2345-6789"}'
```

## What Gets Encrypted

Currently encrypted:

| Data | Storage | AAD |
|------|---------|-----|
| Environment variables (`env_variables` table) | SQLite | `env:{key}` |

The `encrypted` column in `env_variables` tracks whether each row is
encrypted (1) or plaintext (0). When the vault is unsealed, new env vars
are written encrypted. When sealed or uninitialized, they are written as
plaintext.

```admonish info title="Planned"
KeyStore (provider API keys in `provider_keys.json`) and TokenStore
(OAuth tokens in `credentials.json`) are currently sync/file-based and
cannot easily call async vault methods. Encryption for these stores is
planned after an async refactor.
```

## Vault Guard Middleware

When the vault is in the **Sealed** state, a middleware layer blocks API
requests (except auth and bootstrap endpoints) with `423 Locked`:

```json
{"error": "vault is sealed", "status": "sealed"}
```

This prevents the application from serving stale or unreadable data when
the vault needs to be unlocked.

The guard does **not** block when the vault is **Uninitialized** — there's
nothing to protect yet, and the application needs to function normally for
initial setup.

Allowed through regardless of vault state:

- `/api/auth/*` — authentication endpoints (including vault unlock)
- `/api/gon` — server-injected bootstrap data
- Non-API routes — static assets, HTML pages, health check

## API Endpoints

All vault endpoints are under `/api/auth/vault/` and require no session
(they are on the public auth allowlist):

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/auth/vault/status` | Returns `{"status": "uninitialized"\|"sealed"\|"unsealed"\|"disabled"}` |
| `POST` | `/api/auth/vault/unlock` | Unseal with password: `{"password": "..."}` |
| `POST` | `/api/auth/vault/recovery` | Unseal with recovery key: `{"recovery_key": "..."}` |

Error responses:

| Status | Meaning |
|--------|---------|
| `200` | Success |
| `423 Locked` | Bad password or recovery key |
| `404` | Vault not available (feature disabled) |
| `500` | Internal error |

## Frontend Integration

The vault status is included in the gon data (`window.__MOLTIS__`) on
every page load:

```js
import * as gon from "./gon.js";
const vaultStatus = gon.get("vault_status");
// "uninitialized" | "sealed" | "unsealed" | "disabled"
```

Live updates are available via `gon.onChange("vault_status", callback)`.

### Locked-vault banners

When `vault_status` is `sealed`, the UI shows an info banner:

- In the main app shell (`index.html`): a banner linking to
  **Settings > Encryption** for manual unlock.
- On the login page (`/login`): a banner that explains the vault is locked
  and will unlock after successful sign-in.

### Onboarding and localhost

The onboarding wizard's Security step explains that setting a password
also enables the encryption vault for stored secrets. The password
selection card explicitly says: "Set a password and enable the encryption
vault for stored secrets."

On localhost, where authentication is optional, the subtitle mentions
that setting a password enables the vault — giving users a reason to
set one even when network security is not a concern.

When a password is set during first-time setup, the server returns a
`recovery_key` field in the JSON response. The onboarding wizard shows an
interstitial screen with:

- A success message ("Password set and vault initialized")
- The recovery key in a monospace `<code>` block with `select-all` for
  easy selection
- A **Copy** button using the Clipboard API
- A warning that the key will not be shown again
- A **Continue** button to proceed to the next onboarding step

In **Settings > Authentication**, setting a password for the first time
also returns a `recovery_key`. The page keeps the user on Settings long
enough to copy it, then shows a **Continue to sign in** action when the
new password makes authentication mandatory.

Passkey-only setup does not trigger vault initialization (no password to
derive a KEK from), so the recovery key screen is never shown in that flow.

### Vault status in Settings > Encryption

When the vault feature is compiled in, an **Encryption** tab appears in
Settings (under the Security group). It tells the user their API keys
and secrets are encrypted before being stored, and that the vault locks
on restart and unlocks on login.

| Vault state | Badge | What it means |
|-------------|-------|---------------|
| **Unsealed** | Green ("Unlocked") | Your API keys and secrets are encrypted in the database. Everything is working. |
| **Sealed** | Amber ("Locked") | Log in or unlock below to access your encrypted keys. |
| **Uninitialized** | Gray ("Off") | Set a password in Authentication settings to start encrypting your stored keys. |

When the vault is **sealed**, both unlock forms are shown in the same
panel (password and recovery key, separated by an "or" divider). Submitting
calls `POST /api/auth/vault/unlock` or `POST /api/auth/vault/recovery`,
then refreshes gon data to update the status badge.

### Encrypted badges on environment variables

Each environment variable in **Settings > Environment** shows a badge
indicating its encryption status:

| Badge | Style | Meaning |
|-------|-------|---------|
| **Encrypted** | Green (`.provider-item-badge.configured`) | Value is encrypted at rest by the vault |
| **Plaintext** | Gray (`.provider-item-badge.muted`) | Value is stored in cleartext |

A status note at the top of the section explains the current vault state:

- **Unlocked**: "Your keys are stored encrypted."
- **Locked**: "Encrypted keys can't be read — sandbox commands won't
  work." Links to Encryption settings to unlock.
- **Not set up**: "Set a password to encrypt your stored keys." Links
  to Authentication settings.

## Disabling the Vault

To compile without vault support, disable the `vault` feature:

```bash
cargo build --no-default-features --features "web-ui,tls"
```

When the feature is disabled, all vault code is compiled out via
`#[cfg(feature = "vault")]`. Environment variables are stored as plaintext,
and the vault API endpoints return 404.

## Cryptographic Details

| Parameter | Value |
|-----------|-------|
| AEAD cipher | XChaCha20-Poly1305 (192-bit nonce, 256-bit key) |
| KDF | Argon2id |
| Argon2id memory | 64 MiB |
| Argon2id iterations | 3 |
| Argon2id parallelism | 1 |
| DEK size | 256 bits |
| Nonce generation | Random per encryption (24 bytes) |
| AAD | Context string per data type (e.g. `env:MY_KEY`) |
| Key wrapping | XChaCha20-Poly1305 (KEK encrypts DEK) |
| Recovery key | 128-bit random, 32-char alphanumeric encoding (8 groups of 4) |

The nonce is prepended to the ciphertext and stored as base64. AAD
(Additional Authenticated Data) binds each ciphertext to its context,
preventing an attacker from swapping encrypted values between keys.

## Database Schema

The vault uses a single metadata table:

```sql
CREATE TABLE IF NOT EXISTS vault_metadata (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),
    version              INTEGER NOT NULL DEFAULT 1,
    kdf_salt             TEXT    NOT NULL,
    kdf_params           TEXT    NOT NULL,
    wrapped_dek          TEXT    NOT NULL,
    recovery_wrapped_dek TEXT,
    recovery_key_hash    TEXT,
    created_at           TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT    NOT NULL DEFAULT (datetime('now'))
);
```

The `CHECK (id = 1)` constraint ensures only one row exists — the vault
is a singleton per database.

The gateway migration adds the `encrypted` column to `env_variables`:

```sql
ALTER TABLE env_variables ADD COLUMN encrypted INTEGER NOT NULL DEFAULT 0;
```
