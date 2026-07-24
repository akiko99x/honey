# Security policy

## Supported versions

| Version | Security fixes |
|---|---|
| `0.0.x` | Supported |
| older snapshots | Not supported |

Until honey reaches `1.0.0`, upgrades can include operational or configuration
changes. Read the release notes and keep a verified PostgreSQL backup plus the
matching off-box `HONEY_SECRET_KEY` before upgrading.

## Reporting a vulnerability

Do not open a public issue, discussion or pull request for a vulnerability,
credential leak or deployment-specific secret. Use
[GitHub private vulnerability reporting](https://github.com/akiko99x/honey/security/advisories/new).
If that form is unavailable, contact the repository owner privately and wait
for a secure reporting channel before sharing technical details.

Include the affected version, impact, reproduction conditions and the smallest
safe proof of concept. Never attach production environment files, databases,
certificates, private keys, subscription URLs, enrollment bundles or live
credentials. Replace all deployment-specific values with synthetic examples.

The maintainers aim to acknowledge a complete report within seven days, assess
severity and scope, and coordinate a fix and disclosure timeline with the
reporter. These are response targets, not a warranty.

## Operational security

A public repository contains source and examples only. A honey deployment must
keep `/etc/honey`, PostgreSQL data, backups, PKI, `HONEY_SECRET_KEY`, API keys,
subscription tokens and core configuration outside the repository. If a live
secret is committed, revoke or rotate it first; deleting the file or rewriting
Git history does not make the leaked value safe again.
