-- Keep values inserted outside the REST API safe for protobuf conversion and
-- core config generation as well.
ALTER TABLE nodes
  ADD CONSTRAINT nodes_grpc_port_chk CHECK (grpc_port BETWEEN 1 AND 65535);

ALTER TABLE inbounds
  ADD CONSTRAINT inbounds_listen_port_chk CHECK (listen_port BETWEEN 1 AND 65535),
  ADD CONSTRAINT inbounds_reality_tls_chk CHECK (NOT reality OR tls_enabled);

ALTER TABLE users
  ADD CONSTRAINT users_traffic_limit_chk CHECK (traffic_limit_bytes >= 0);
