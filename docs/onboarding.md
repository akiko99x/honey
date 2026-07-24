# First-run onboarding

Honey shows a setup checklist on Overview after an administrator signs in. The
checklist is not a wizard state machine and has no completion flag: `GET
/onboarding` derives every step from the current PostgreSQL resources.

The full operator sequence is:

1. register an owned domain;
2. connect a node;
3. create an inbound;
4. create a user;
5. reveal and share the user's subscription.

`subscription` is complete when at least one in-scope user has a current token
encrypted at rest (`subscription_token_enc`). Older users migrated from the
hash-only token format remain incomplete until their subscription is rotated.
Deleting a resource makes its step incomplete again; restoring the database or
creating the resource advances it without browser-local state.

The response contains only counts converted to booleans and safe UI metadata:

```json
{
  "completed": 3,
  "total": 5,
  "steps": [
    {
      "key": "domain",
      "label": "Register a domain",
      "description": "Add an owned hostname for TLS or a public endpoint.",
      "complete": true,
      "route": "domains",
      "action": "add-domain"
    }
  ]
}
```

Resellers receive only `user` and `subscription`. Their counts are restricted
to users they own; node, inbound and domain facts never enter the response.
The panel exposes Overview, Users and Subscriptions for this role and hides all
infrastructure navigation and setup actions.

The unauthenticated panel is a dedicated full-page sign-in screen. Ctrl-K is
available only after authentication and filters pages/actions by role. It can
continue setup, create common resources, jump to entities, refresh state and
open permitted account tools. The inbound wizard includes inline help for SNI,
REALITY target, network transports, transport Host and Hysteria2 `hop_ports`.

`scripts/e2e-api.sh` verifies the derived transition
domain → node → inbound → user → subscription against a disposable master. It
creates and removes a synthetic `.invalid` managed domain with the rest of its
smoke resources; do not run that lifecycle script against production.
