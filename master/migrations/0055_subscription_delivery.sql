-- Safe defaults for frequent profile refresh and broadly compatible XHTTP
-- clients. Existing operator choices are intentionally preserved on upgrade.
INSERT INTO app_settings (key, value) VALUES
  ('profile_update_interval_hours', '1'),
  ('subscription_fallback_base_url', ''),
  ('subscription_client_profiles', '{"happ-android":{"xhttp_mode":"packet-up","fingerprint":"chrome"},"happ-desktop":{"xhttp_mode":"packet-up","fingerprint":"chrome"},"karing":{"xhttp_mode":"packet-up","fingerprint":"chrome"},"generic":{"xhttp_mode":"packet-up","fingerprint":"chrome"}}')
ON CONFLICT (key) DO NOTHING;
