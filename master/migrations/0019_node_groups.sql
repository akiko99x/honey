-- Node groups as the access model (replaces per-inbound assignment).
--
-- A node in NO group is universal (reachable by every user). A node in one or
-- more groups is reachable only by users granted access to a shared group.
-- The old per-inbound `inbound_users` m2m is removed.

CREATE TABLE node_groups (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name       text NOT NULL UNIQUE,
  is_default boolean NOT NULL DEFAULT false,
  note       text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT node_groups_name_chk CHECK (name <> '')
);
CREATE UNIQUE INDEX node_groups_one_default ON node_groups(is_default) WHERE is_default;
CREATE TRIGGER node_groups_set_updated_at BEFORE UPDATE ON node_groups
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- node ↔ group (m2m). No rows for a node ⇒ that node is universal.
CREATE TABLE node_group_members (
  node_id  uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  group_id uuid NOT NULL REFERENCES node_groups(id) ON DELETE CASCADE,
  PRIMARY KEY (node_id, group_id)
);

-- user ↔ group access (m2m).
CREATE TABLE user_group_access (
  user_id  uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  group_id uuid NOT NULL REFERENCES node_groups(id) ON DELETE CASCADE,
  PRIMARY KEY (user_id, group_id)
);
CREATE INDEX user_group_access_group_idx ON user_group_access(group_id);
CREATE INDEX node_group_members_group_idx ON node_group_members(group_id);

-- seed a default group and grant every existing user access to it, so once a
-- node is added to the default group existing users immediately reach it.
INSERT INTO node_groups (name, is_default, note)
  VALUES ('default', true, 'auto-created default group');
INSERT INTO user_group_access (user_id, group_id)
  SELECT u.id, g.id FROM users u, node_groups g WHERE g.is_default;

-- existing nodes stay ungrouped (universal) so current reachability is preserved
-- after dropping per-inbound assignment.
DROP TABLE inbound_users;
