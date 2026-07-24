# Issues / health cockpit

Honey derives a read-only fleet health snapshot from state already maintained
by the master. It does not add another alert database or expose raw upstream
errors. Open **Issues** in the panel or call authenticated `GET /issues`.

Each issue also exposes normalized `labels`. User issues carry user labels,
node and certificate issues carry node labels, and inbound issues combine their
own labels with their node labels. This lets the Issues page reuse the same
label filters and personal saved views as the resource tables. Labels are
metadata only; see [labels and saved views](labels-saved-views.md).

## Response contract

The response contains `generated_at`, severity `counts`, and an `issues` array.
Each issue has a stable `id`, `severity`, an existing Honey diagnostic `code`,
`kind`, safe title/message copy, entity identifiers, optional `node_id`, an
optional safe `action`, and `detected_at`. Results are sorted critical first,
then warning and info, and are stable within each severity.

Current actions are:

- `retry_push`: opens the existing config preview before an operator applies it;
- `probe_inbound`: runs the existing on-demand reachability probe;
- `verify_domain`: runs the existing DNS/TLS managed-domain check.

The API never returns node push error text, reach errors, domain check errors,
credentials, certificate fingerprints, or private keys as part of an issue.
Use the authenticated logs and entity drill-in when deeper diagnosis is needed.

## Conditions

| Condition | Severity | Code | Clearing condition |
| --- | --- | --- | --- |
| Enabled node unseen for more than two minutes | critical | `M0409` | agent heartbeat returns |
| Last desired-config push failed | critical | `M0406` | a later push succeeds |
| Enabled inbound's latest probe failed | warning | `M1501` | a later probe succeeds |
| Managed domain unchecked | warning | `M1302` | run Verify |
| Managed-domain DNS failed | critical | `M1302` | verification succeeds |
| Managed-domain port 443 unreachable | warning | `M1302` | verification succeeds |
| Managed-domain certificate invalid | critical | `M1301` | valid certificate is served |
| Managed-domain certificate expires within 14 days | warning | `M1301` | certificate is renewed |
| Enrolled node has no currently usable certificate | critical | `M0810` | enroll a valid replacement |
| Earliest active agent certificate expires within 14 days | warning | `M0810` | enroll a longer-lived replacement |
| User expired or quota-reached | warning | `M0703` | renew/reset/roll over quota |
| User intentionally disabled | info | `M0703` | enable or delete the user |

Disabled nodes and inbounds do not create availability issues. A node with no
certificate inventory keeps the documented legacy CA-valid compatibility and
does not create certificate noise. Historical revoked/expired certificates do
not create an issue while another valid certificate is active.

## Prometheus

The authenticated `/metrics` output adds current gauge values:

```text
honey_issues{severity="critical"} 0
honey_issues{severity="warning"} 2
honey_issues{severity="info"} 1
```

Page on `critical`, alert on persistent `warning`, and normally retain `info`
only for dashboards. Because this is a current-state gauge, use the Logs/audit
views when event history is required.
