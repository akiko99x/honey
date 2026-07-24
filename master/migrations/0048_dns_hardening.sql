-- client-side DNS hardening shipped inside the subscription's sing-box config.
-- dns_doh: a DoH resolver URL (empty = no DNS section, current behaviour).
-- dns_fakeip: route A/AAAA queries through a FakeIP pool (anti DNS leak/ad).
-- dns_block_plain: drop outgoing :53 so plaintext DNS can't leak around the DoH.
ALTER TABLE routing_profiles ADD COLUMN dns_doh         text    NOT NULL DEFAULT '';
ALTER TABLE routing_profiles ADD COLUMN dns_fakeip      boolean NOT NULL DEFAULT false;
ALTER TABLE routing_profiles ADD COLUMN dns_block_plain boolean NOT NULL DEFAULT false;
