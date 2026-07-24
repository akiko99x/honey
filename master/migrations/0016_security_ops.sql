-- P1 §11 security & operations.

-- TOTP two-factor for admins. Secret is stored encrypted at rest (like other
-- secrets) and only becomes active once totp_enabled is set (after the operator
-- confirms a first code).
ALTER TABLE admins
  ADD COLUMN totp_secret  text,
  ADD COLUMN totp_enabled boolean NOT NULL DEFAULT false;

-- Periodic (rolling) quota windows: a user's traffic_limit_bytes can apply per
-- day/week; a scheduler resets usage at quota_reset_at and advances it.
ALTER TABLE users
  ADD COLUMN quota_interval text NOT NULL DEFAULT 'none',
  ADD COLUMN quota_reset_at timestamptz,
  ADD CONSTRAINT users_quota_interval_chk CHECK (quota_interval IN ('none', 'daily', 'weekly'));

-- Optional admin IP allowlist. Empty table = allow from anywhere.
CREATE TABLE admin_ip_allowlist (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  cidr       text NOT NULL UNIQUE,
  note       text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now()
);
