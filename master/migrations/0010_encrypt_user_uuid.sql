-- encrypt the vless/vmess uuid at rest. it's a credential like the password, so
-- store it as text (ciphertext). existing values convert to their plaintext
-- string form; `honey-master reencrypt` wraps them afterwards. new rows get an
-- encrypted uuid from the app; the default is a plaintext fallback only.
ALTER TABLE users ALTER COLUMN uuid DROP DEFAULT;
ALTER TABLE users ALTER COLUMN uuid TYPE text USING uuid::text;
ALTER TABLE users ALTER COLUMN uuid SET DEFAULT gen_random_uuid()::text;
