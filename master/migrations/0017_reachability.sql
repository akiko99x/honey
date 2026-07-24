-- P2 §12 reachability & failover.

-- Additional public addresses/IPs a node is reachable at, so a subscription can
-- offer several failover targets per inbound and rotate between them.
ALTER TABLE nodes
  ADD COLUMN extra_addresses text[] NOT NULL DEFAULT '{}';

-- Per-inbound data-plane reachability, distinct from "agent online" (control
-- plane). NULL = unknown/unchecked; probed by the master and/or reported by an
-- external vantage-point checker.
ALTER TABLE inbounds
  ADD COLUMN reachable        boolean,
  ADD COLUMN reach_checked_at timestamptz,
  ADD COLUMN reach_error      text;
