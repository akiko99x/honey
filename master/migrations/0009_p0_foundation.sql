-- P0 production foundation: human administrators and sessions, durable
-- desired/applied node state, enrollment credentials, certificate inventory,
-- and an append-only audit trail.

CREATE TABLE admins (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  username      text NOT NULL,
  password_hash text NOT NULL,
  role          text NOT NULL DEFAULT 'admin',
  enabled       boolean NOT NULL DEFAULT true,
  last_login_at timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT admins_username_chk CHECK (length(trim(username)) BETWEEN 2 AND 96),
  CONSTRAINT admins_role_chk CHECK (role IN ('owner', 'admin', 'operator', 'viewer'))
);

CREATE UNIQUE INDEX admins_username_lower_uniq ON admins(lower(username));
CREATE TRIGGER admins_set_updated_at BEFORE UPDATE ON admins
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE admin_sessions (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  admin_id    uuid NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
  token_hash  bytea NOT NULL UNIQUE,
  expires_at  timestamptz NOT NULL,
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  user_agent  text,
  remote_addr text,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX admin_sessions_admin_idx ON admin_sessions(admin_id);
CREATE INDEX admin_sessions_expiry_idx ON admin_sessions(expires_at);

CREATE TABLE audit_events (
  id            bigserial PRIMARY KEY,
  actor_admin_id uuid REFERENCES admins(id) ON DELETE SET NULL,
  actor_name    text,
  action        text NOT NULL,
  resource_type text NOT NULL,
  resource_id  text,
  request_id   uuid NOT NULL DEFAULT gen_random_uuid(),
  remote_addr  text,
  details      jsonb NOT NULL DEFAULT '{}',
  created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX audit_events_created_idx ON audit_events(created_at DESC);
CREATE INDEX audit_events_resource_idx ON audit_events(resource_type, resource_id);

ALTER TABLE nodes
  ADD COLUMN desired_spec_hash text,
  ADD COLUMN applied_spec_hash text,
  ADD COLUMN applied_at timestamptz,
  ADD COLUMN last_push_at timestamptz,
  ADD COLUMN last_push_status text,
  ADD COLUMN last_push_message text;

CREATE TABLE node_push_events (
  id            bigserial PRIMARY KEY,
  node_id       uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  desired_hash  text NOT NULL,
  applied_hash  text,
  source        text NOT NULL,
  status        text NOT NULL,
  message       text,
  actor_admin_id uuid REFERENCES admins(id) ON DELETE SET NULL,
  started_at    timestamptz NOT NULL DEFAULT now(),
  finished_at   timestamptz,
  CONSTRAINT node_push_events_status_chk CHECK (status IN ('started', 'applied', 'unchanged', 'failed'))
);

CREATE INDEX node_push_events_node_idx ON node_push_events(node_id, started_at DESC);

CREATE TABLE node_enrollment_tokens (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  node_id       uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  token_hash    bytea NOT NULL UNIQUE,
  created_by    uuid REFERENCES admins(id) ON DELETE SET NULL,
  expires_at    timestamptz NOT NULL,
  claimed_at    timestamptz,
  revoked_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT node_enrollment_expiry_chk CHECK (expires_at > created_at)
);

CREATE INDEX node_enrollment_node_idx ON node_enrollment_tokens(node_id, created_at DESC);

CREATE TABLE node_certificates (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  node_id         uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  serial_number   text NOT NULL UNIQUE,
  fingerprint_sha256 text NOT NULL UNIQUE,
  subject         text NOT NULL,
  not_before      timestamptz NOT NULL,
  not_after       timestamptz NOT NULL,
  issued_at       timestamptz NOT NULL DEFAULT now(),
  revoked_at      timestamptz,
  replaced_by     uuid REFERENCES node_certificates(id) ON DELETE SET NULL,
  CONSTRAINT node_certificates_dates_chk CHECK (not_after > not_before)
);

CREATE INDEX node_certificates_node_idx ON node_certificates(node_id, issued_at DESC);
