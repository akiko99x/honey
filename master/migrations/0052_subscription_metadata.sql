-- Optional per-user Happ profile metadata. NULL means use the operator default.
ALTER TABLE users
    ADD COLUMN subscription_description TEXT;

ALTER TABLE users
    ADD CONSTRAINT users_subscription_description_length
    CHECK (subscription_description IS NULL OR char_length(subscription_description) BETWEEN 1 AND 200);
