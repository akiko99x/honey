ALTER TABLE inbounds
  ADD COLUMN IF NOT EXISTS udp_idle_timeout text NOT NULL DEFAULT '60s';
