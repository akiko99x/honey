-- Owned-domains registry for the data plane. Distinct from panel_domains (the
-- panel Host allowlist): these are domains you own and point at nodes/CDN, and
-- inbounds/public endpoints pick from them. SNI and REALITY dest stay free-text
-- (borrowed domains you don't own), so they are NOT constrained to this list.
CREATE TABLE managed_domains (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  host            text NOT NULL,
  node_id         uuid REFERENCES nodes(id) ON DELETE SET NULL,
  proxied         boolean NOT NULL DEFAULT false,   -- fronted by a CDN (e.g. Cloudflare)
  notes           text NOT NULL DEFAULT '',
  last_checked_at timestamptz,
  dns_ok          boolean NOT NULL DEFAULT false,
  resolved_ips    text[] NOT NULL DEFAULT '{}',
  reachable_443   boolean NOT NULL DEFAULT false,
  check_error     text,
  created_at      timestamptz NOT NULL DEFAULT now(),
  updated_at      timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT managed_domains_host_uniq UNIQUE (host),
  CONSTRAINT managed_domains_host_lower_chk CHECK (host = lower(host)),
  CONSTRAINT managed_domains_host_shape_chk CHECK (
    host <> '' AND host !~ '[/:[:space:]]'
  )
);

CREATE INDEX managed_domains_node_idx ON managed_domains(node_id);

CREATE TRIGGER managed_domains_set_updated_at BEFORE UPDATE ON managed_domains
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
