# Changelog

All notable changes to honey are documented here. The project uses semantic
version tags and is currently pre-stable.

## [Unreleased]

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
- Interactive Ubuntu/Debian bootstrap, checksummed release installer, backups,
  restore rehearsal and opt-in GitHub self-update.
- GitHub Actions for tests, database recovery, secret scanning and release
  packaging.

[Unreleased]: https://github.com/akiko99x/honey/compare/v0.0.5...HEAD
[0.0.5]: https://github.com/akiko99x/honey/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/akiko99x/honey/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/akiko99x/honey/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/akiko99x/honey/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/akiko99x/honey/compare/v0.0.0...v0.0.1
[0.0.0]: https://github.com/akiko99x/honey/releases/tag/v0.0.0
