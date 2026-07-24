-- one-time recovery codes for administrators with TOTP enabled.
-- Only SHA-256 digests are stored; the generated codes are shown once.
CREATE TABLE admin_recovery_codes (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    admin_id uuid NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
    code_hash bytea NOT NULL UNIQUE,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX admin_recovery_codes_admin_idx
    ON admin_recovery_codes (admin_id, used_at);

ALTER TABLE admin_login_events
    DROP CONSTRAINT IF EXISTS admin_login_events_outcome_chk;

ALTER TABLE admin_login_events
    ADD CONSTRAINT admin_login_events_outcome_chk
    CHECK (outcome IN ('success', 'bad_credentials', 'bad_totp', 'bad_recovery_code', 'ip_denied', 'rate_limited'));
