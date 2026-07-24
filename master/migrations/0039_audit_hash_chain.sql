-- audit tamper-evidence: each event carries a SHA-256 hash chaining it to the
-- previous entry, so deletion or modification of any past event is detectable.
-- existing rows keep NULL (legacy, pre-chain); the chain starts from the next
-- recorded event.
ALTER TABLE audit_events ADD COLUMN entry_hash bytea;
