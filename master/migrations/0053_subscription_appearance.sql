-- v0.1.0 subscription presentation controls.
--
-- Per-user NULL group and "inherit" traffic policy resolve through the global
-- app settings. The permanent subscription URL itself uses users.id, so no
-- additional plaintext bearer secret needs to be stored.
ALTER TABLE users
    ADD COLUMN subscription_group TEXT;

ALTER TABLE users
    ADD CONSTRAINT users_subscription_group_length
    CHECK (subscription_group IS NULL OR char_length(subscription_group) BETWEEN 1 AND 40);

ALTER TABLE users
    ADD COLUMN subscription_traffic_policy TEXT NOT NULL DEFAULT 'inherit';

ALTER TABLE users
    ADD CONSTRAINT users_subscription_traffic_policy
    CHECK (subscription_traffic_policy IN ('inherit', 'auto', 'always', 'never'));
