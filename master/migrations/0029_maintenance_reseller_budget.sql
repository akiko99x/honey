-- node maintenance mode + reseller traffic budget & commission.
--
-- maintenance drains a node from subscriptions without disabling its control
-- plane: the agent keeps its config, but the node's inbounds drop out of every
-- user's subscription so clients move to other nodes in their groups. distinct
-- from `enabled=false` (which stops the node entirely).
ALTER TABLE nodes ADD COLUMN maintenance boolean NOT NULL DEFAULT false;

-- reseller-level total traffic budget (sum over the reseller's own users; 0 =
-- unlimited) and a commission percentage kept for billing/payout reporting.
ALTER TABLE admins
  ADD COLUMN traffic_limit_bytes bigint  NOT NULL DEFAULT 0,
  ADD COLUMN commission_percent  integer NOT NULL DEFAULT 0;
