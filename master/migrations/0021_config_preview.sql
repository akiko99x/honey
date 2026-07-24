-- Sanitized structural snapshot of the last successfully applied NodeSpec.
-- Credentials, private keys, paths, hosts, and extra_json are intentionally
-- excluded by the application-level summary serializer.
ALTER TABLE nodes
    ADD COLUMN applied_spec_summary JSONB;
