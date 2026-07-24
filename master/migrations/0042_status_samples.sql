-- uptime sampling for the public status page: a low-frequency online/offline
-- probe per enabled node, sampled from last_seen freshness (DB-only, no agent
-- round-trip). ratio over a window = uptime %. maintenance nodes are not sampled
-- so scheduled drains don't count as outage. pruned to a rolling 7-day window.
CREATE TABLE node_status_samples (
  id         bigserial PRIMARY KEY,
  node_id    uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  online     boolean NOT NULL,
  sampled_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX node_status_samples_node_idx ON node_status_samples (node_id, sampled_at DESC);
CREATE INDEX node_status_samples_time_idx ON node_status_samples (sampled_at);
