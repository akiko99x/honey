-- runtime-editable operator settings. a tiny key/value store the master reads
-- live (loops re-read each iteration; handlers per request), so an operator can
-- tune reconcile cadence, retention and inbound defaults from the panel instead
-- of CLI/env + restart. unknown keys are ignored; missing keys fall back to code
-- defaults, so the table is safe to be empty.
CREATE TABLE app_settings (
  key        text PRIMARY KEY,
  value      text NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);
