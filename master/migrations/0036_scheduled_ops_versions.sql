-- deferred operations run by a scheduler loop, and per-entity change history
-- with snapshots for revert.

-- a future action on a node/user/inbound (enable a plan from a date, disable at
-- a cutoff, push at a quiet hour, …). the scheduler executes due pending rows.
CREATE TABLE scheduled_operations (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  resource_type text NOT NULL,
  resource_id   uuid NOT NULL,
  action        text NOT NULL,
  run_at        timestamptz NOT NULL,
  status        text NOT NULL DEFAULT 'pending',
  result        text,
  created_by    uuid REFERENCES admins(id) ON DELETE SET NULL,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT scheduled_ops_type_chk CHECK (resource_type IN ('node', 'user', 'inbound')),
  CONSTRAINT scheduled_ops_status_chk CHECK (status IN ('pending', 'done', 'failed', 'canceled'))
);

CREATE INDEX scheduled_ops_due_idx ON scheduled_operations (run_at) WHERE status = 'pending';

-- an immutable snapshot of an entity captured on each create/update, so the
-- panel can show change history and revert to a prior version.
CREATE TABLE entity_versions (
  id            bigserial PRIMARY KEY,
  resource_type text NOT NULL,
  resource_id   uuid NOT NULL,
  snapshot      jsonb NOT NULL,
  actor         text,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX entity_versions_lookup_idx ON entity_versions (resource_type, resource_id, id DESC);
