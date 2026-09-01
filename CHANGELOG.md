# Changelog

All notable changes to honey are documented here. The project uses semantic
version tags and is currently pre-stable.

## [Unreleased]

## [0.1.5] - 2026-09-01

### Added

- `honey-mcp`, an official stdio MCP server that exposes every protected panel
  operation through separate discovery, read, write, delete and fleet-status
  tools while preserving Honey API-key RBAC, auditing and request IDs.
- Automatic MCP operation discovery from the panel router, backed by a coverage
  test so new panel routes cannot silently disappear from the MCP surface.
- Linux and Windows x86_64 MCP release binaries and secure API-key-file support.

### Security

- MCP requests are pinned to one configured Honey origin, reject path traversal
  and embedded query strings, bound response sizes and require an exact
  confirmation phrase for DELETE calls.

## [0.1.4] - 2026-08-30

### Added

- Master reachability monitoring and rollout preflight now verify Hysteria2
  and TUIC endpoints through credential-free QUIC Version Negotiation over
  UDP, instead of leaving them permanently unchecked.

### Changed

- The panel now distinguishes a pending QUIC result from a confirmed failure
  and explains which transport-specific probe was used.

## [0.1.3] - 2026-08-18

### Added

- Public subscriptions now support an operator-configured HTTPS fallback
  origin, allowing imports and QR codes to be delivered through a reserve
  server while the master remains the source of truth.
- The public page has a new mobile-first Neko VPN installer with device and app
  selection, compact account details, and an accessible liquid-glass design.

### Changed

- The default subscription refresh interval is now one hour and remains
  editable globally from runtime settings.
- XHTTP client profiles default to the widely supported `packet-up` mode and
  `chrome` uTLS fingerprint.

### Fixed

- Canonical `/v2ray`, raw-link, alias, and User-Agent-tailored subscriptions now
  apply their client compatibility profile instead of leaking the inbound's
  older `auto`/`qq` values to clients.

## [0.1.2] - 2026-08-18

### Fixed

- Happ one-tap imports now use a native, percent-encoded deep link to preserve
  the complete `/v2ray` subscription URL across browsers and operating systems.
- Docker and bootstrap installations now use Hysteria 2.12.1, which restores
  clients promptly after stale QUIC sessions caused by sleep or network changes.

## [0.1.1] - 2026-07-29

### Added

- Official Hysteria 2 server core managed by honey-agent.
- Per-inbound UDP idle timeout setting for Hysteria2.
- Native Hysteria traffic-statistics integration.

### Changed

- Existing Hysteria2 inbounds are automatically routed from sing-box to the official Hysteria server.


## [0.1.0] - 2026-07-28

### Added

- Permanent per-user UUID subscription links that remain available without
  rotating protocol credentials or the optional revocable subscription token.
- Global and per-user subscription presentation controls for client title,
  description, group and traffic-row visibility.
- Configurable Happ Android, Happ Desktop, Karing and generic Xray compatibility
  profiles with per-client XHTTP mode and uTLS fingerprint overrides.
- An operator subscription preview showing effective metadata, client profile
  URLs, endpoint transport settings and generation warnings.

### Changed

- Credential and revocable-link rotation are now explicitly optional secondary
  actions; the panel shows the permanent subscription link by default.
- Subscription update interval and support metadata are managed alongside the
  global subscription appearance settings.
- Unlimited users no longer receive `Subscription-Userinfo` by default when the
  global traffic policy is `auto`, avoiding an unnecessary infinity row in Happ.

## [0.0.13] - 2026-07-27

### Changed

- VLESS subscriptions now default to the `qq` uTLS fingerprint, with all
  Happ-supported fingerprint choices available in the inbound forms.
- XHTTP inbounds now default to `auto` mode.
- Hysteria2 authentication is generated consistently as `username:password`
  in node configs and every subscription format.

## [0.0.12] - 2026-07-26

### Fixed

- Restored Happ one-tap subscription imports.
- Improved VLESS XHTTP compatibility with packet-up defaults.
- Clarified UDP inbound status reporting.

## [0.0.11] - 2026-07-26

### Fixed

- TUIC sing-box users now include both the required UUID and password, so a
  combined Hysteria2/TUIC configuration validates and starts correctly.
- Inbound flow values are validated consistently by the API and the guided
  form now offers only supported VLESS choices.
- Creating users, nodes, and inbounds now returns to the matching list after
  the result dialog is dismissed.
- Docker and bootstrap local-node enrollment now use the detected public IPv4
  address, with `HONEY_NODE_ADDRESS` available for NAT or restricted hosts.

## [0.0.10] - 2026-07-26

### Fixed

- Xray ACME now uses an explicit HTTP-01 flow instead of autocert's
  TLS-ALPN-first behavior, so it works behind the Docker Caddy gateway.
- Application-level agent apply failures no longer mark a healthy node offline.
- Docker upgrades restart Caddy after replacing the bind-mounted configuration.

## [0.0.9] - 2026-07-26

### Fixed

- Docker clean installation now retains the root-only bootstrap password file
  through optional local-node enrollment, then removes it immediately after
  the authenticated session is created.

### Changed

- README and Docker runbooks now describe the complete one-script clean install,
  prompts, health verification, pinned versions and upgrade path.

## [0.0.8] - 2026-07-26

### Fixed

- Docker master and migration containers retain only `SETUID` and `SETGID`
  while starting so the entrypoint can drop from root to the unprivileged
  `honey` account after reading root-only Compose secrets.
- The Docker master entrypoint now imports the root-only at-rest encryption
  key before dropping privileges instead of passing its unreadable file path
  to the `honey` process.

## [0.0.7] - 2026-07-26

### Fixed

- Docker master and migration containers read root-only Compose secret files
  before dropping to the unprivileged `honey` account.
- Docker PKI bootstrap now runs through the privilege-dropping master
  entrypoint so named-volume files remain owned by `honey`.

## [0.0.6] - 2026-07-26

### Added

- Production Docker Compose deployment for PostgreSQL, master, agent, Caddy
  and scheduled PostgreSQL backups.
- Tagged GHCR images for master, agent and backup, published by the release
  workflow.
- Docker installer, upgrade command, backup/restore helpers and clean-server
  deployment runbook.
- Configurable Honey-managed Xray ACME paths and challenge addresses for
  container deployments.

### Changed

- Docker Compose is the preferred clean-install path; systemd remains available
  as a legacy and recovery fallback.

## [0.0.5] - 2026-07-26

### Added

- Honey-managed Xray ACME certificates with persistent cache, HTTP-01
  challenge handling, atomic PEM export and automatic renewal/reload.
- Xray ACME support in the inbound wizard; it uses the local challenge gateway
  on `127.0.0.1:9080` and keeps manual certificate paths available.

### Changed

- Xray ACME metadata is translated by the agent and never passed through to
  Xray's inbound schema.

## [0.0.4] - 2026-07-26

### Added

- Global or per-user Happ subscription title and announcement templates, with
  traffic/expiry placeholders and a Telegram support button.
- Full-screen user creation/editing with generated passwords and prefix-based
  batch issuance.

### Changed

- Xray TLS supports manual certificate paths, REALITY, or Honey-managed ACME.

### Fixed

- Bootstrap emits explicit domain-specific HTTP and HTTPS Caddy blocks so
  HTTP-01 challenge forwarding is not shadowed by automatic redirects.

## [0.0.3] - 2026-07-25

### Fixed

- Local-node bootstrap now keeps its session cookie usable over the loopback
  HTTP connection, allowing node creation and enrollment to complete.
- Bootstrap formats the generated Caddyfile before validation and installation.
- The hardened master service can write its dedicated PKI directory while
  issuing enrolled node certificates.

## [0.0.2] - 2026-07-25

### Fixed

- Bootstrap now writes a valid multiline Caddy global options block when an
  ACME contact email is provided.

## [0.0.1] - 2026-07-25

### Added

- Custom Happ subscription titles and descriptions.
- Per-inbound Happ profile names and country flags.

### Fixed

- sing-box ACME state now uses a persistent writable directory compatible with
  the hardened systemd service.
- Bootstrap Caddy configuration now forwards HTTP-01 challenges to sing-box,
  and ACME challenge selection is explicit.

## [0.0.0] - 2026-07-25

### Added

- Rust master with PostgreSQL, panel, REST API and multi-node orchestration.
- Go agent and one-time mTLS enrollment client.
- sing-box and Xray lifecycle management with candidate validation and rollback.
- Serve and NAT-friendly dial transports.
- VLESS/REALITY and Hysteria2 release smoke coverage, subscription exports,
  traffic accounting, quotas, expiry and operational diagnostics.
- Docker Compose deployment plus the legacy Ubuntu/Debian bootstrap,
  checksummed release installer, backups, restore rehearsal and opt-in GitHub
  self-update.
- GitHub Actions for tests, database recovery, secret scanning and release
  packaging.

[Unreleased]: https://github.com/akiko99x/honey/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/akiko99x/honey/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/akiko99x/honey/compare/v0.0.13...v0.1.0
[0.0.13]: https://github.com/akiko99x/honey/compare/v0.0.12...v0.0.13
[0.0.12]: https://github.com/akiko99x/honey/compare/v0.0.11...v0.0.12
[0.0.11]: https://github.com/akiko99x/honey/compare/v0.0.10...v0.0.11
[0.0.10]: https://github.com/akiko99x/honey/compare/v0.0.9...v0.0.10
[0.0.9]: https://github.com/akiko99x/honey/compare/v0.0.8...v0.0.9
[0.0.8]: https://github.com/akiko99x/honey/compare/v0.0.7...v0.0.8
[0.0.7]: https://github.com/akiko99x/honey/compare/v0.0.6...v0.0.7
[0.0.6]: https://github.com/akiko99x/honey/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/akiko99x/honey/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/akiko99x/honey/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/akiko99x/honey/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/akiko99x/honey/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/akiko99x/honey/compare/v0.0.0...v0.0.1
[0.0.0]: https://github.com/akiko99x/honey/releases/tag/v0.0.0
