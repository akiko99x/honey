-- honey master schema, v1.
-- model: nodes -> inbounds (protocols) -> users (m2m via inbound_users).
-- a NodeSpec for the agent is assembled from a node + its inbounds + their users.

CREATE EXTENSION IF NOT EXISTS pgcrypto; -- gen_random_uuid()

-- keeps updated_at honest on every UPDATE.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
  NEW.updated_at := now();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ---------------------------------------------------------------------------
-- nodes: one server running a honey agent.
-- ---------------------------------------------------------------------------
CREATE TABLE nodes (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name            text NOT NULL UNIQUE,
  address         text NOT NULL,                    -- host/ip the agent is reachable at
  grpc_port       integer NOT NULL DEFAULT 8443,
  transport       text NOT NULL DEFAULT 'serve',    -- serve | dial | both
  enabled         boolean NOT NULL DEFAULT true,
  last_seen       timestamptz,
  agent_version   text,
  singbox_version text,
  created_at      timestamptz NOT NULL DEFAULT now(),
  updated_at      timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT nodes_transport_chk CHECK (transport IN ('serve', 'dial', 'both'))
);

CREATE TRIGGER nodes_set_updated_at BEFORE UPDATE ON nodes
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- inbounds: one protocol listener on a node (vless, hysteria2, ...).
-- many inbounds per node = many protocols in one sing-box process.
-- ---------------------------------------------------------------------------
CREATE TABLE inbounds (
  id                        uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  node_id                   uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  tag                       text NOT NULL,
  type                      text NOT NULL,          -- vless | hysteria2 | vmess | trojan | shadowsocks | tuic
  listen                    text NOT NULL DEFAULT '::',
  listen_port               integer NOT NULL,
  flow                      text NOT NULL DEFAULT '', -- vless flow, e.g. xtls-rprx-vision

  -- tls
  tls_enabled               boolean NOT NULL DEFAULT false,
  server_name               text,
  cert_path                 text,
  key_path                  text,

  -- reality (subset of tls)
  reality                   boolean NOT NULL DEFAULT false,
  reality_private_key       text,
  reality_short_ids         text[] NOT NULL DEFAULT '{}',
  reality_handshake_server  text,
  reality_handshake_port    integer,

  extra                     jsonb NOT NULL DEFAULT '{}', -- merged verbatim into the inbound
  enabled                   boolean NOT NULL DEFAULT true,
  created_at                timestamptz NOT NULL DEFAULT now(),
  updated_at                timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT inbounds_tag_uniq  UNIQUE (node_id, tag),
  CONSTRAINT inbounds_port_uniq UNIQUE (node_id, listen_port)
);

CREATE INDEX inbounds_node_idx ON inbounds(node_id);

CREATE TRIGGER inbounds_set_updated_at BEFORE UPDATE ON inbounds
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- users: a subscriber. carries both a uuid (vless/vmess) and a password
-- (hysteria2/trojan/ss), so the same identity works across protocols.
-- ---------------------------------------------------------------------------
CREATE TABLE users (
  id                   uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  username             text NOT NULL UNIQUE,
  uuid                 uuid NOT NULL DEFAULT gen_random_uuid(),
  password             text NOT NULL,
  enabled              boolean NOT NULL DEFAULT true,
  traffic_limit_bytes  bigint NOT NULL DEFAULT 0,     -- 0 = unlimited
  used_traffic_bytes   bigint NOT NULL DEFAULT 0,
  expires_at           timestamptz,                   -- null = never
  created_at           timestamptz NOT NULL DEFAULT now(),
  updated_at           timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER users_set_updated_at BEFORE UPDATE ON users
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- inbound_users: which users are provisioned on which inbounds (m2m).
-- ---------------------------------------------------------------------------
CREATE TABLE inbound_users (
  inbound_id  uuid NOT NULL REFERENCES inbounds(id) ON DELETE CASCADE,
  user_id     uuid NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
  created_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (inbound_id, user_id)
);

CREATE INDEX inbound_users_user_idx ON inbound_users(user_id);
