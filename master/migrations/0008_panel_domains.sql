-- Domains and URL prefixes allowed to serve the embedded admin panel.
CREATE TABLE panel_domains (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  host       text NOT NULL,
  base_path  text NOT NULL DEFAULT '/panel',
  enabled    boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT panel_domains_host_path_uniq UNIQUE (host, base_path),
  CONSTRAINT panel_domains_host_lower_chk CHECK (host = lower(host)),
  CONSTRAINT panel_domains_host_shape_chk CHECK (
    host <> '' AND host !~ '[/:[:space:]]'
  ),
  CONSTRAINT panel_domains_path_chk CHECK (
    base_path ~ '^/[A-Za-z0-9._~/-]+$'
    AND base_path <> '/'
    AND right(base_path, 1) <> '/'
    AND base_path !~ '//|(^|/)\.\.?(/|$)'
  )
);

CREATE INDEX panel_domains_enabled_idx ON panel_domains(enabled, host);

CREATE TRIGGER panel_domains_set_updated_at BEFORE UPDATE ON panel_domains
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
