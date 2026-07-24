-- named, scoped API keys for automation / integrations, replacing reliance on a
-- single shared bearer token. only the SHA-256 of each key is stored; the scope
-- is one of the existing roles so the same rank checks apply. keys can expire and
-- be revoked, and their last use is tracked.
CREATE TABLE api_keys (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name         text NOT NULL,
  key_hash     bytea NOT NULL UNIQUE,
  role         text NOT NULL DEFAULT 'viewer',
  created_by   uuid REFERENCES admins(id) ON DELETE SET NULL,
  last_used_at timestamptz,
  expires_at   timestamptz,
  revoked_at   timestamptz,
  created_at   timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT api_keys_role_chk CHECK (role IN ('owner', 'admin', 'operator', 'viewer'))
);

CREATE INDEX api_keys_hash_idx ON api_keys (key_hash);
