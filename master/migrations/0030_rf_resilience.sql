-- rf-resilience automation: a vantage-checker fleet with history & consensus,
-- a direct→CDN failover host, and an SNI pool for safe reactive rotation.

-- append-only log of reachability verdicts from external vantage checkers (and
-- optionally the master). the effective inbounds.reachable is recomputed from
-- the recent vantage consensus so one region-block drains the endpoint.
CREATE TABLE reachability_reports (
  id          bigserial PRIMARY KEY,
  inbound_id  uuid NOT NULL REFERENCES inbounds(id) ON DELETE CASCADE,
  source      text NOT NULL,
  reachable   boolean NOT NULL,
  latency_ms  integer,
  error       text,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX reachability_reports_inbound_idx
  ON reachability_reports (inbound_id, created_at DESC);

ALTER TABLE inbounds
  -- when a direct endpoint is confirmed blocked, the subscription fronts it via
  -- this CDN host instead of dropping it (only meaningful for ws/http/xhttp).
  ADD COLUMN fallback_host text,
  -- owned SNIs the operator may rotate an inbound through when a value is blocked.
  ADD COLUMN sni_pool text[] NOT NULL DEFAULT '{}';
