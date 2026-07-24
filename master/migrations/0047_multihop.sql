-- multihop / cascade: an entry inbound can route its users' traffic to another
-- inbound (the exit) instead of egressing directly, so traffic exits from a
-- second node (e.g. enter in RU, exit abroad). The entry node gets a sing-box
-- outbound to the exit plus a route rule; the exit inbound gains a dedicated
-- chain credential as one of its users. Both cores must be sing-box.
ALTER TABLE inbounds ADD COLUMN upstream_inbound_id uuid
  REFERENCES inbounds(id) ON DELETE SET NULL;
-- stable credential the entry node uses to authenticate to the exit inbound.
ALTER TABLE inbounds ADD COLUMN chain_uuid     text;
ALTER TABLE inbounds ADD COLUMN chain_password text;  -- encrypted at rest

CREATE INDEX inbounds_upstream_idx ON inbounds (upstream_inbound_id)
  WHERE upstream_inbound_id IS NOT NULL;
