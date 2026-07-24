-- first-class network transports and extra tls options on inbounds.
-- previously these lived only in `extra` (jsonb) and never reached client links.
ALTER TABLE inbounds
  ADD COLUMN network text NOT NULL DEFAULT 'tcp',
  ADD COLUMN transport_path text,
  ADD COLUMN transport_host text,
  ADD COLUMN transport_service_name text,
  ADD COLUMN transport_mode text,
  ADD COLUMN ech boolean NOT NULL DEFAULT false,
  ADD COLUMN utls_fingerprint text,
  ADD COLUMN shadowtls_handshake_server text,
  ADD COLUMN shadowtls_handshake_port integer;

ALTER TABLE inbounds
  ADD CONSTRAINT inbounds_network_chk CHECK (network IN
    ('tcp','ws','grpc','http','h2','httpupgrade','xhttp','quic','mkcp'));

ALTER TABLE inbounds
  ADD CONSTRAINT inbounds_shadowtls_port_chk CHECK
    (shadowtls_handshake_port IS NULL OR shadowtls_handshake_port BETWEEN 1 AND 65535);
