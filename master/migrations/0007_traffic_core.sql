-- track traffic per core: a user shared across sing-box and xray on one node
-- has two independent counter sources, so the core is part of the key.
ALTER TABLE node_user_traffic
  ADD COLUMN core text NOT NULL DEFAULT 'singbox';

ALTER TABLE node_user_traffic
  DROP CONSTRAINT node_user_traffic_pkey;

ALTER TABLE node_user_traffic
  ADD PRIMARY KEY (node_id, user_id, core);
