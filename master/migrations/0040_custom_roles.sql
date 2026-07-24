-- custom RBAC: granular per-admin permissions beyond the fixed rank roles.
-- a custom role is a matrix of domain -> level (0 none, 1 read, 2 write). when an
-- admin is assigned a custom role, that matrix is authoritative for authorization
-- instead of the rank ladder. admins with no custom role are unaffected.
CREATE TABLE custom_roles (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name        text NOT NULL UNIQUE,
  permissions jsonb NOT NULL DEFAULT '{}',
  created_at  timestamptz NOT NULL DEFAULT now(),
  updated_at  timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE admins
  ADD COLUMN custom_role_id uuid REFERENCES custom_roles(id) ON DELETE SET NULL;
