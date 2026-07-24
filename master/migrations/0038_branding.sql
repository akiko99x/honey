-- white-label branding: a singleton row applied to the panel, the public
-- subscription page and the status page. all values are plain text/toggles
-- (rendered escaped) so there is no raw-HTML injection surface.
CREATE TABLE branding (
  id                 smallint PRIMARY KEY DEFAULT 1,
  brand_name         text NOT NULL DEFAULT 'honey',
  logo_url           text NOT NULL DEFAULT '',
  accent_color       text NOT NULL DEFAULT '',
  support_url        text NOT NULL DEFAULT '',
  support_text       text NOT NULL DEFAULT '',
  footer_text        text NOT NULL DEFAULT '',
  sub_welcome        text NOT NULL DEFAULT '',
  sub_show_imports   boolean NOT NULL DEFAULT true,
  sub_show_downloads boolean NOT NULL DEFAULT true,
  sub_show_endpoints boolean NOT NULL DEFAULT true,
  updated_at         timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT branding_singleton CHECK (id = 1)
);

INSERT INTO branding (id) VALUES (1) ON CONFLICT DO NOTHING;
