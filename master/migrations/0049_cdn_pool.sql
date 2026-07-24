-- proactive CDN rotation by latency. cdn_pool holds candidate fronting hosts
-- (CDN hostnames) for a ws/http inbound; a background pass measures the TCP
-- connect latency to each and points transport_host at the fastest reachable
-- one, so clients front through the best edge (rotation "by ping"). This is
-- proactive — the existing sni_pool rotation only fires on a confirmed block.
ALTER TABLE inbounds ADD COLUMN cdn_pool text[] NOT NULL DEFAULT '{}';
