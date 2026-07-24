-- Counter epochs make samples idempotent across agent restarts. A new epoch is
-- treated as a baseline instead of adding an already-seen live connection.
ALTER TABLE node_user_traffic
  ADD COLUMN last_epoch text NOT NULL DEFAULT '';
