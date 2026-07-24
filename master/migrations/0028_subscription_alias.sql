-- short, human-friendly subscription alias so a user's link can be
-- /s/<alias> instead of the long uuid token. case-insensitively unique;
-- optional (null = no alias, only the token URL works).
ALTER TABLE users ADD COLUMN subscription_alias text;

CREATE UNIQUE INDEX users_subscription_alias_lower_uniq
  ON users (lower(subscription_alias))
  WHERE subscription_alias IS NOT NULL;
