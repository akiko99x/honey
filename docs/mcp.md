# Honey MCP server

`honey-mcp` is a local stdio Model Context Protocol server that translates MCP
tool calls into authenticated Honey panel API requests. It never connects to
PostgreSQL and cannot bypass panel authorization: every request still passes
through the named API-key role checks, custom role rules, IP allowlist and audit
request-ID middleware.

## Tools

| Tool | State | Purpose |
|---|---:|---|
| `honey_discover` | read-only | Search every method/path in the protected panel router. |
| `honey_status` | read-only | Liveness, readiness, public status, issues, nodes and optional detailed telemetry. |
| `honey_read` | read-only | Call any GET route, including config previews, logs, history and analytics. |
| `honey_write` | writes | Call any POST, PUT or PATCH route. |
| `honey_delete` | destructive | Call DELETE only when `confirm` exactly equals `DELETE <path>`. |

The operation catalog is compiled from the same router source as the panel. A
unit test requires the complete protected router to remain discoverable, so a
new panel action cannot silently disappear from MCP.

## Authentication

Create a dedicated named key in **Settings → API keys**. Choose the narrowest
role that fits the automation:

- `viewer` for statuses and diagnostics;
- `operator` for routine pushes, rotations, labels and traffic resets;
- `admin` for nodes, users, inbounds and most configuration;
- `owner` only when MCP must also manage global settings, updates, admins,
  custom roles, API keys or configuration import/export.

Store the one-time token in a user-only file with a trailing newline allowed.
Do not place it in command arguments, repository files or MCP configuration.

```bash
install -d -m 700 ~/.config/honey
install -m 600 /dev/null ~/.config/honey/mcp-api-key
# Enter the token with a local secret editor, then:
honey-mcp --base-url https://panel.example.com/honey \
  --api-key-file ~/.config/honey/mcp-api-key
```

For a Docker deployment, the packaged installer places `honey-mcp` in
`/usr/local/bin`. A secure remote Codex setup can launch it over the existing
SSH connection and keep the API key only on the Honey host:

```bash
codex mcp add honey -- \
  ssh -T root@panel-host \
  /usr/local/bin/honey-mcp \
  --base-url http://127.0.0.1:8080 \
  --api-key-file /root/.config/honey/mcp-api-key
```

Pin the SSH host key and use a dedicated restricted SSH key. The stdio process
writes MCP messages only to stdout; diagnostics and connection failures are
returned as tool results without exposing the bearer token.

## Request safety

- The base URL must be one absolute HTTP(S) origin without query or fragment.
- Tool paths must start with exactly one slash and cannot contain a scheme,
  query, fragment or dot segments, preventing requests to another origin.
- Query values are scalars or arrays of scalars; bodies are JSON.
- Responses default to a 1 MiB cap and can never exceed 10 MiB.
- DELETE uses a separate tool and exact per-path confirmation.
- Honey API errors preserve HTTP status and `x-request-id` for audit lookup.

MCP client approvals are still recommended for both `honey_write` and
`honey_delete`, especially when the key has `admin` or `owner` scope.
