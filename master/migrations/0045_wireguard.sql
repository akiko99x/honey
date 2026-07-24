-- WireGuard / AmneziaWG data-plane. This is a separate path from sing-box/xray:
-- the agent runs a real wg/awg interface per wg_interfaces row, with one peer
-- per user that has access to the node. Keys are Curve25519 (standard base64);
-- private keys are encrypted at rest. AmneziaWG obfuscation params live in jsonb.
CREATE TABLE wg_interfaces (
  id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  node_id        uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  name           text NOT NULL,
  listen_port    int  NOT NULL,
  private_key    text NOT NULL,           -- enc at rest
  public_key     text NOT NULL,
  address_cidr   text NOT NULL,           -- e.g. 10.7.0.0/24 (server takes .1)
  dns            text NOT NULL DEFAULT '1.1.1.1',
  mtu            int  NOT NULL DEFAULT 1420,
  amnezia        boolean NOT NULL DEFAULT false,
  amnezia_params jsonb   NOT NULL DEFAULT '{}',
  endpoint_host  text,                     -- overrides node.address for clients
  enabled        boolean NOT NULL DEFAULT true,
  created_at     timestamptz NOT NULL DEFAULT now(),
  updated_at     timestamptz NOT NULL DEFAULT now(),
  UNIQUE (node_id, listen_port)
);

CREATE TABLE wg_peers (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  interface_id uuid NOT NULL REFERENCES wg_interfaces(id) ON DELETE CASCADE,
  user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  private_key  text NOT NULL,             -- enc at rest (panel-generated client key)
  public_key   text NOT NULL,
  address      text NOT NULL,             -- /32 host from the pool
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (interface_id, user_id),
  UNIQUE (interface_id, address)
);

CREATE INDEX wg_interfaces_node_idx ON wg_interfaces (node_id);
CREATE INDEX wg_peers_user_idx ON wg_peers (user_id);
