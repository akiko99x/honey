-- Complete the schema used by the multi-core agent and add revocable public
-- subscription identifiers. The token is a credential and is never returned
-- by ordinary user list/get endpoints.
ALTER TABLE inbounds
  ADD COLUMN core text NOT NULL DEFAULT 'singbox',
  ADD COLUMN reality_public_key text,
  ADD CONSTRAINT inbounds_core_chk CHECK (core IN ('singbox', 'xray'));

ALTER TABLE users
  ADD COLUMN subscription_token uuid NOT NULL DEFAULT gen_random_uuid(),
  ADD CONSTRAINT users_subscription_token_uniq UNIQUE (subscription_token);

CREATE INDEX inbounds_core_idx ON inbounds(core);
