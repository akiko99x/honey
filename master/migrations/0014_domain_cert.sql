-- Certificate probe results for managed domains: the notAfter read from the
-- TLS endpoint on :443 and whether it is currently valid (not expired).
ALTER TABLE managed_domains
  ADD COLUMN cert_not_after timestamptz,
  ADD COLUMN cert_ok        boolean NOT NULL DEFAULT false;
