-- Hourly traffic history built from the same restart-safe deltas that feed
-- users.used_traffic_bytes.  Quota resets clear the live accumulator, not this
-- history, so period analytics remain stable across billing/quota windows.
CREATE TABLE traffic_usage_hourly (
  bucket       timestamptz NOT NULL,
  node_id      uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  core         text NOT NULL CHECK (core IN ('singbox', 'xray')),
  up_bytes     bigint NOT NULL DEFAULT 0 CHECK (up_bytes >= 0),
  down_bytes   bigint NOT NULL DEFAULT 0 CHECK (down_bytes >= 0),
  sample_count bigint NOT NULL DEFAULT 0 CHECK (sample_count >= 0),
  PRIMARY KEY (bucket, node_id, user_id, core)
);

CREATE INDEX traffic_usage_hourly_bucket_idx
  ON traffic_usage_hourly (bucket DESC);

CREATE INDEX traffic_usage_hourly_user_bucket_idx
  ON traffic_usage_hourly (user_id, bucket DESC);

CREATE INDEX traffic_usage_hourly_node_bucket_idx
  ON traffic_usage_hourly (node_id, bucket DESC);
