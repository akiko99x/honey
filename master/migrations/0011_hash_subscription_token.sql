-- store only the SHA-256 of the subscription token, never the token itself, so
-- a db leak can't hand out working subscription URLs. existing tokens keep
-- working: their hash (of the uuid text) is exactly what the app computes on
-- lookup. the plaintext column is dropped.
ALTER TABLE users ADD COLUMN subscription_token_hash bytea;

UPDATE users SET subscription_token_hash = digest(subscription_token::text, 'sha256');

ALTER TABLE users ALTER COLUMN subscription_token_hash SET NOT NULL;
ALTER TABLE users
  ADD CONSTRAINT users_subscription_token_hash_uniq UNIQUE (subscription_token_hash);

ALTER TABLE users DROP COLUMN subscription_token;
