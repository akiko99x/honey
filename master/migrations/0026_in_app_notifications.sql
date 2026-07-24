-- Persistent in-panel alert history. Events are global operational facts;
-- read state is private to each administrator account.

CREATE TABLE system_notifications (
  id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  event_type       text NOT NULL,
  dedupe_key       text NOT NULL,
  severity         text NOT NULL,
  code             text NOT NULL,
  title            text NOT NULL,
  body             text NOT NULL,
  resource_type    text,
  resource_id      text,
  occurrence_count integer NOT NULL DEFAULT 1,
  last_seen_at     timestamptz NOT NULL DEFAULT now(),
  created_at       timestamptz NOT NULL DEFAULT now(),

  CONSTRAINT system_notifications_event_chk CHECK (char_length(event_type) BETWEEN 1 AND 64),
  CONSTRAINT system_notifications_key_chk CHECK (char_length(dedupe_key) BETWEEN 1 AND 256),
  CONSTRAINT system_notifications_severity_chk CHECK (severity IN ('critical','warning','info')),
  CONSTRAINT system_notifications_code_chk CHECK (char_length(code) BETWEEN 1 AND 16),
  CONSTRAINT system_notifications_title_chk CHECK (char_length(title) BETWEEN 1 AND 160),
  CONSTRAINT system_notifications_body_chk CHECK (char_length(body) BETWEEN 1 AND 1024),
  CONSTRAINT system_notifications_resource_type_chk CHECK (
    resource_type IS NULL OR char_length(resource_type) BETWEEN 1 AND 32
  ),
  CONSTRAINT system_notifications_resource_id_chk CHECK (
    resource_id IS NULL OR char_length(resource_id) BETWEEN 1 AND 128
  ),
  CONSTRAINT system_notifications_occurrence_chk CHECK (occurrence_count > 0)
);

CREATE INDEX system_notifications_created_idx
  ON system_notifications(created_at DESC);
CREATE INDEX system_notifications_key_created_idx
  ON system_notifications(dedupe_key, created_at DESC);
CREATE INDEX system_notifications_severity_created_idx
  ON system_notifications(severity, created_at DESC);

CREATE TABLE admin_notification_reads (
  admin_id       uuid NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
  notification_id uuid NOT NULL REFERENCES system_notifications(id) ON DELETE CASCADE,
  read_at        timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (admin_id, notification_id)
);

CREATE INDEX admin_notification_reads_notification_idx
  ON admin_notification_reads(notification_id);
