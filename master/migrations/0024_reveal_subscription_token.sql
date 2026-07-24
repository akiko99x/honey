-- optional reveal: keep the subscription token encrypted at rest (XChaCha20 via
-- the app's secret key) alongside its SHA-256 hash. the hash stays the lookup
-- key (0011); the encrypted copy only lets an authenticated admin re-display the
-- current link without rotating it. existing users have no encrypted copy (their
-- plaintext was dropped in 0011) — they stay reveal-less until the next rotation.
--
-- trade-off vs the hash-only posture: a leak of BOTH the database and the secret
-- key would expose current tokens. the token is a 122-bit random uuid used only
-- for a revocable subscription URL, so this is an accepted, deliberate inversion.
ALTER TABLE users ADD COLUMN subscription_token_enc text;
