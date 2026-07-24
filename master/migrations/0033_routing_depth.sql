-- routing depth: content-filter / parental blocking + custom domain rules on
-- routing profiles. all emitted into both sing-box `route` and Clash `rules`.
ALTER TABLE routing_profiles
  ADD COLUMN block_adult     boolean NOT NULL DEFAULT false,
  ADD COLUMN block_gambling  boolean NOT NULL DEFAULT false,
  -- custom per-profile domain lists (suffix match)
  ADD COLUMN blocked_domains text[] NOT NULL DEFAULT '{}',
  ADD COLUMN direct_domains  text[] NOT NULL DEFAULT '{}',
  ADD COLUMN proxy_domains   text[] NOT NULL DEFAULT '{}';
