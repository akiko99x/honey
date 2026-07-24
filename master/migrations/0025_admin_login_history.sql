-- Active-session management and bounded login history. Session credentials
-- remain one-way hashes in admin_sessions and are never exposed by the API.

CREATE TABLE admin_login_events (
  id          bigserial PRIMARY KEY,
  admin_id    uuid REFERENCES admins(id) ON DELETE SET NULL,
  username    text NOT NULL,
  outcome     text NOT NULL,
  remote_addr text,
  user_agent  text,
  created_at  timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT admin_login_events_username_chk CHECK (char_length(username) BETWEEN 1 AND 96),
  CONSTRAINT admin_login_events_outcome_chk CHECK (
    outcome IN ('success', 'bad_credentials', 'bad_totp', 'ip_denied', 'rate_limited')
  ),
  CONSTRAINT admin_login_events_remote_chk CHECK (remote_addr IS NULL OR char_length(remote_addr) <= 128),
  CONSTRAINT admin_login_events_user_agent_chk CHECK (user_agent IS NULL OR char_length(user_agent) <= 256)
);

CREATE INDEX admin_login_events_admin_created_idx
  ON admin_login_events(admin_id, created_at DESC);
CREATE INDEX admin_login_events_created_idx
  ON admin_login_events(created_at DESC);
