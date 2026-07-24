-- multiple named subscription links per user (work / home / phone), each an
-- independently revocable token resolving to the same user config; and per-app
-- routing rules (geosite category -> direct/proxy/block) on routing profiles.

CREATE TABLE user_subscriptions (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name       text NOT NULL,
  token_hash bytea NOT NULL UNIQUE,
  token_enc  text,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX user_subscriptions_user_idx ON user_subscriptions (user_id);

-- [{"geosite":"telegram","action":"direct"}, {"geosite":"category-porn","action":"block"}]
ALTER TABLE routing_profiles ADD COLUMN app_rules jsonb NOT NULL DEFAULT '[]';
