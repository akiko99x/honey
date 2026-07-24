-- traffic shaping: per-inbound bandwidth caps (Mbps), 0 = unlimited. Applied
-- natively by sing-box for hysteria2 inbounds (up_mbps/down_mbps drive its
-- congestion control / rate). vless/vmess/trojan cores have no per-user speed
-- limiter, so an inbound tier is the shaping unit (use it as a speed plan).
ALTER TABLE inbounds ADD COLUMN up_mbps   int NOT NULL DEFAULT 0;
ALTER TABLE inbounds ADD COLUMN down_mbps int NOT NULL DEFAULT 0;
