-- P2 §14 notifications.

-- Outbound notification channels: generic webhook, Discord/Slack incoming
-- webhooks, or a Telegram bot (target = "bot_token:chat_id"). `events` filters
-- which categories are sent; empty = all.
CREATE TABLE notify_channels (
  id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name       text NOT NULL,
  kind       text NOT NULL,
  target     text NOT NULL,
  events     text[] NOT NULL DEFAULT '{}',
  enabled    boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT notify_channels_kind_chk CHECK (kind IN ('webhook','discord','slack','telegram'))
);

CREATE TRIGGER notify_channels_set_updated_at BEFORE UPDATE ON notify_channels
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Telegram chat allowlist for the interactive bot. `admin` chats can run admin
-- commands; `user` chats get self-service only.
CREATE TABLE telegram_chats (
  chat_id    bigint PRIMARY KEY,
  role       text NOT NULL DEFAULT 'user',
  note       text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT telegram_chats_role_chk CHECK (role IN ('admin','user'))
);
