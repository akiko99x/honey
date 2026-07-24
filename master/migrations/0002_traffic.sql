-- per-(node,user) accumulated traffic. monotonic and restart-safe:
-- up_bytes/down_bytes accumulate; last_up/last_down hold the last cumulative
-- counter seen from the core, so deltas survive master restarts.
CREATE TABLE node_user_traffic (
  node_id     uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  up_bytes    bigint NOT NULL DEFAULT 0,   -- accumulated
  down_bytes  bigint NOT NULL DEFAULT 0,
  last_up     bigint NOT NULL DEFAULT 0,   -- last cumulative-from-core seen
  last_down   bigint NOT NULL DEFAULT 0,
  updated_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (node_id, user_id)
);

CREATE INDEX node_user_traffic_user_idx ON node_user_traffic(user_id);
