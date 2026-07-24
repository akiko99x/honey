-- Versioned routing profiles delivered inside subscriptions. A profile is a set
-- of high-level routing toggles that each client output (sing-box, Clash)
-- translates into its native rules, so edits propagate on the next refresh.
CREATE TABLE routing_profiles (
  id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name           text NOT NULL UNIQUE,
  version        integer NOT NULL DEFAULT 1,
  block_ads      boolean NOT NULL DEFAULT false,     -- geosite category-ads -> reject
  direct_private boolean NOT NULL DEFAULT true,      -- bypass LAN / private ranges
  direct_geosite text[]  NOT NULL DEFAULT '{}',      -- e.g. {cn, ru} -> direct
  direct_geoip   text[]  NOT NULL DEFAULT '{}',      -- e.g. {cn, ru} -> direct
  final_proxy    boolean NOT NULL DEFAULT true,      -- MATCH -> proxy (else direct)
  is_default     boolean NOT NULL DEFAULT false,
  notes          text    NOT NULL DEFAULT '',
  created_at     timestamptz NOT NULL DEFAULT now(),
  updated_at     timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT routing_profiles_name_chk CHECK (name <> '')
);

-- at most one default profile.
CREATE UNIQUE INDEX routing_profiles_one_default ON routing_profiles(is_default) WHERE is_default;

CREATE TRIGGER routing_profiles_set_updated_at BEFORE UPDATE ON routing_profiles
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- a user can be pinned to a profile; NULL means "use the default profile".
ALTER TABLE users ADD COLUMN routing_profile_id uuid REFERENCES routing_profiles(id) ON DELETE SET NULL;
