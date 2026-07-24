-- multi-master HA. Every instance serves the API; exactly one holds a lease and
-- runs the singleton background loops (reconcile, stats, quota, schedule,
-- monitors, acme, bot). Election is a single-row lease in Postgres — no extra
-- infrastructure. A leader that cannot renew steps down, so a partitioned
-- instance stops acting rather than racing the new leader.
CREATE TABLE ha_leader (
  id          text PRIMARY KEY,
  holder      uuid        NOT NULL,
  acquired_at timestamptz NOT NULL DEFAULT now(),
  renewed_at  timestamptz NOT NULL DEFAULT now(),
  expires_at  timestamptz NOT NULL,
  CONSTRAINT ha_leader_single CHECK (id = 'master')
);

-- liveness roster for the panel (who is up, who leads).
CREATE TABLE ha_instances (
  instance_id uuid PRIMARY KEY,
  hostname    text        NOT NULL,
  version     text        NOT NULL,
  started_at  timestamptz NOT NULL DEFAULT now(),
  last_seen   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ha_instances_seen_idx ON ha_instances (last_seen DESC);
