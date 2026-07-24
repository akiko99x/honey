# Secret backends (at-rest master key)

honey encrypts sensitive values at rest — user credentials (UUID/password),
REALITY private keys, subscription tokens — with XChaCha20-Poly1305 under a
process-wide 32-byte master key (base64). Historically that key came only from
`HONEY_SECRET_KEY`. The master now resolves it from one of several backends so
the key need not sit in a plain environment variable.

## Precedence

The first configured source wins. A configured-but-failing source is a **hard
error** (the master refuses to start). No configuration at all leaves at-rest
encryption **off** (dev plaintext mode — a startup warning is logged).

| # | Trigger env | Backend | Notes |
|---|-------------|---------|-------|
| 1 | `HONEY_SECRET_KEY` | `env` | Direct base64 value. Backward-compatible default. |
| 2 | `HONEY_SECRET_KEY_FILE` | `file` | Path to a file holding the base64 key. Docker/K8s secrets, systemd `LoadCredential=`. |
| 3 | `HONEY_VAULT_ADDR` | `vault` | HashiCorp Vault KV read over HTTP (see below). |
| 4 | `HONEY_SECRET_KEY_COMMAND` | `command` | Shell command whose stdout is the key. Universal hatch: AWS/GCP secret managers, `pass`, `sops`, … |

The active backend is shown read-only in **Settings → Secrets & encryption** and
logged at startup (`M0114`).

## Generating a key

```
honey-master keygen        # prints a fresh base64 32-byte key
```

Store it in your chosen backend, then start the master.

## HashiCorp Vault

Reads a KV secret and pulls one field out of it.

```
HONEY_VAULT_ADDR=https://vault.internal:8200
HONEY_VAULT_TOKEN=<token with read on the path>
HONEY_VAULT_PATH=honey            # logical path under the mount
HONEY_VAULT_MOUNT=secret          # optional, default: secret
HONEY_VAULT_FIELD=key             # optional, default: key
HONEY_VAULT_KV_VERSION=2          # optional, default: 2 (set 1 for legacy KV v1)
```

KV v2 request: `GET {addr}/v1/{mount}/data/{path}`, field read from `data.data.<field>`.
KV v1 request: `GET {addr}/v1/{mount}/{path}`, field read from `data.<field>`.
The token is sent as `X-Vault-Token`; a non-2xx response aborts startup. Token
renewal / AppRole login are out of scope — supply an already-valid token (e.g.
via the Vault agent sidecar) or use the command backend for custom auth.

Example seeding the secret:

```
vault kv put secret/honey key="$(honey-master keygen)"
```

## Command backend

Any command that prints the base64 key to stdout:

```
HONEY_SECRET_KEY_COMMAND='aws secretsmanager get-secret-value --secret-id honey/master --query SecretString --output text'
```

Run via `sh -c` (POSIX) / `cmd /C` (Windows). Non-zero exit or empty output
aborts startup.

## Rotation

Key rotation still uses the CLI (`honey-master rekey`, `HONEY_SECRET_KEY_OLD` →
`HONEY_SECRET_KEY`), which re-encrypts every stored secret — including named
subscription tokens. Rotate the value in your backend, then run `rekey` with the
old and new keys. See the deployment runbook.
