-- per-user device limit for anti-sharing. counts distinct concurrent source
-- IPs observed on the live Clash /connections snapshot; 0 = unlimited. There is
-- no first-party client, so this is IP-based (a "device" = a source address),
-- not a hardware fingerprint. Enforcement (alert vs close excess connections) is
-- a runtime setting; see monitor::device_limit_loop.
ALTER TABLE users ADD COLUMN device_limit int NOT NULL DEFAULT 0;
