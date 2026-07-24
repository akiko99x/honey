-- Runtime-tunable protection for public subscription documents. These values
-- are deliberately generous for ordinary client polling and may be changed
-- live from Settings without restarting the master.
INSERT INTO app_settings (key, value) VALUES
  ('subscription_guard_enabled', 'true'),
  ('subscription_guard_max_requests', '120'),
  ('subscription_guard_window_secs', '60'),
  ('subscription_guard_block_secs', '300')
ON CONFLICT (key) DO NOTHING;
