-- Operator-controlled title shown by subscription clients (Happ, v2rayN, etc.).
-- NULL preserves the historical username/domain fallback.
ALTER TABLE users
    ADD COLUMN subscription_title TEXT;

ALTER TABLE users
    ADD CONSTRAINT users_subscription_title_length
    CHECK (subscription_title IS NULL OR char_length(subscription_title) BETWEEN 1 AND 25);
