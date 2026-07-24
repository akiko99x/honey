-- managed external services on a node that are NOT sing-box/xray inbounds and
-- run as their own daemon (a separate data-plane, like WireGuard):
--   * mtproto — a Telegram MTProto proxy (mtg) with an ee-secret + fake-TLS host
--   * naive   — a NaiveProxy (Caddy forwardproxy) with a user/password over TLS
-- The agent runs the daemon; the master owns config + generates the client link.
CREATE TABLE node_services (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  node_id     uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
  kind        text NOT NULL,               -- 'mtproto' | 'naive'
  name        text NOT NULL,
  listen_port int  NOT NULL,
  secret      text,                         -- encrypted at rest (secret/password)
  config      jsonb NOT NULL DEFAULT '{}',  -- per-kind extras (fake-TLS host, username, domain)
  enabled     boolean NOT NULL DEFAULT true,
  created_at  timestamptz NOT NULL DEFAULT now(),
  updated_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (node_id, listen_port)
);

CREATE INDEX node_services_node_idx ON node_services (node_id);
