-- resellers (sub-admins): a scoped operator that manages only its own users and
-- can only grant node-groups it was entitled to. builds on the group access
-- model (0019). a reseller never touches nodes/inbounds/settings/other admins.

-- 1. allow the 'reseller' role. the CHECK is recreated (can't ALTER a CHECK in pg).
ALTER TABLE admins DROP CONSTRAINT admins_role_chk;
ALTER TABLE admins ADD CONSTRAINT admins_role_chk
  CHECK (role IN ('owner', 'admin', 'operator', 'viewer', 'reseller'));

-- 2. per-reseller allocation caps (0 = unlimited). max_users bounds how many
--    users a reseller may own; user_traffic_ceiling_bytes bounds the per-user
--    traffic limit a reseller may set on each of its users.
ALTER TABLE admins
  ADD COLUMN max_users                 integer NOT NULL DEFAULT 0,
  ADD COLUMN user_traffic_ceiling_bytes bigint NOT NULL DEFAULT 0;

-- 3. ownership: which admin created a user. null = system/owner-owned (all the
--    existing users). resellers may only see/manage users they created.
ALTER TABLE users
  ADD COLUMN created_by uuid REFERENCES admins(id) ON DELETE SET NULL;

CREATE INDEX users_created_by_idx ON users(created_by);

-- 4. reseller entitlement: the set of groups a reseller may assign to its users.
--    a reseller with no rows here can grant nothing (must be provisioned first).
CREATE TABLE reseller_groups (
  admin_id   uuid NOT NULL REFERENCES admins(id)      ON DELETE CASCADE,
  group_id   uuid NOT NULL REFERENCES node_groups(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (admin_id, group_id)
);

CREATE INDEX reseller_groups_group_idx ON reseller_groups(group_id);
