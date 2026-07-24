-- Serve-mode nodes may use a unique DNS SAN. Legacy nodes keep the shared
-- honey-agent name until their certificate is rotated.
ALTER TABLE nodes
  ADD COLUMN tls_server_name text NOT NULL DEFAULT 'honey-agent',
  ADD CONSTRAINT nodes_tls_server_name_chk CHECK (length(trim(tls_server_name)) > 0);
