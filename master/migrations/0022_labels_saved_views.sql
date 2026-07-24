-- Operational labels and per-admin saved table views. Labels are metadata only:
-- they do not participate in node-group entitlement or generated core specs.

ALTER TABLE nodes
  ADD COLUMN labels text[] NOT NULL DEFAULT '{}',
  ADD CONSTRAINT nodes_labels_count_chk CHECK (cardinality(labels) <= 16);
ALTER TABLE inbounds
  ADD COLUMN labels text[] NOT NULL DEFAULT '{}',
  ADD CONSTRAINT inbounds_labels_count_chk CHECK (cardinality(labels) <= 16);
ALTER TABLE users
  ADD COLUMN labels text[] NOT NULL DEFAULT '{}',
  ADD CONSTRAINT users_labels_count_chk CHECK (cardinality(labels) <= 16);

CREATE INDEX nodes_labels_gin ON nodes USING gin(labels);
CREATE INDEX inbounds_labels_gin ON inbounds USING gin(labels);
CREATE INDEX users_labels_gin ON users USING gin(labels);

CREATE TABLE saved_views (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  admin_id   uuid NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
  name       text NOT NULL,
  resource   text NOT NULL,
  definition jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT saved_views_name_chk CHECK (char_length(name) BETWEEN 1 AND 80),
  CONSTRAINT saved_views_resource_chk CHECK (resource IN ('nodes', 'inbounds', 'users', 'issues')),
  CONSTRAINT saved_views_definition_chk CHECK (jsonb_typeof(definition) = 'object')
);

CREATE UNIQUE INDEX saved_views_admin_resource_name_uq
  ON saved_views(admin_id, resource, lower(name));
CREATE INDEX saved_views_admin_resource_idx ON saved_views(admin_id, resource);
CREATE TRIGGER saved_views_set_updated_at BEFORE UPDATE ON saved_views
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
