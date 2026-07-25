# Changelog

All notable changes to honey are documented here. The project uses semantic
version tags and is currently pre-stable.

## [Unreleased]

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

[Unreleased]: https://github.com/akiko99x/honey/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/akiko99x/honey/compare/v0.0.0...v0.0.1
[0.0.0]: https://github.com/akiko99x/honey/releases/tag/v0.0.0
