-- growth-lite: operator announcements (banner on the subscription page + status
-- page) and per-node cost tracking for infra P&L.
CREATE TABLE announcements (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  title      text NOT NULL,
  body       text NOT NULL DEFAULT '',
  level      text NOT NULL DEFAULT 'info',
  enabled    boolean NOT NULL DEFAULT true,
  created_by uuid REFERENCES admins(id) ON DELETE SET NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT announcements_level_chk CHECK (level IN ('info', 'warning', 'critical'))
);

-- monthly provider cost per node, in minor currency units (cents), 0 = untracked.
ALTER TABLE nodes ADD COLUMN monthly_cost_cents bigint NOT NULL DEFAULT 0;
