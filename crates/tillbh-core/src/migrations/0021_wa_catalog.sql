-- WhatsApp Business catalogue publishing (POS → WhatsApp, one direction).
-- Mappings are scoped to the WhatsApp account they were made with (`account`,
-- the linked number's digits): a different linked account never reuses the
-- remote ids of another; the old rows stay as history. These tables are local
-- to the computer that owns the WhatsApp session (they are not in the row
-- sync), like the WhatsApp session itself. Nothing is queued by this
-- migration: publishing starts only when an administrator runs the first sync.
--
-- status (products):
--   queued          a remote write is due (create / update / hide / delete)
--   syncing         claimed by the catalogue worker (claimed_at = claim time)
--   synced          the remote product matches `fingerprint` and is visible
--   hidden          the remote product exists but is hidden (product archived
--                   or no longer publishable)
--   removed         the POS-owned remote product was deleted
--   not_synced      nothing on WhatsApp (never published, or not publishable)
--   failed          gave up (permanent error, or bounded retries used up);
--                   re-queued when the product changes or on "Retry"
--   remote_missing  the mapped remote product no longer exists on WhatsApp
--                   (deleted there); re-created only on "Retry"
CREATE TABLE wa_catalog_products (
  account         TEXT NOT NULL,
  product_id      TEXT NOT NULL,
  remote_id       TEXT,
  status          TEXT NOT NULL DEFAULT 'not_synced'
                  CHECK (status IN ('queued','syncing','synced','hidden','removed','not_synced','failed','remote_missing')),
  fingerprint     TEXT,
  -- Representation last sent (claim time): a failed product is re-queued
  -- only when this changes, never in a loop.
  attempt_fingerprint TEXT,
  image_hash      TEXT,
  image_url       TEXT,
  adopted         INTEGER NOT NULL DEFAULT 0,
  attempts        INTEGER NOT NULL DEFAULT 0,
  next_at         TEXT,
  claimed_at      TEXT,
  last_attempt_at TEXT,
  last_synced_at  TEXT,
  last_error      TEXT,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL,
  PRIMARY KEY (account, product_id)
);
CREATE UNIQUE INDEX ux_wa_catalog_remote ON wa_catalog_products(account, remote_id) WHERE remote_id IS NOT NULL;
CREATE INDEX ix_wa_catalog_due ON wa_catalog_products(account, status, next_at);

-- POS category → WhatsApp collection. `status` records what the linked
-- connection allows: 'unsupported' when collections cannot be written through
-- it (the case with the current WhatsApp client, see docs/STATUS.md).
CREATE TABLE wa_catalog_collections (
  account        TEXT NOT NULL,
  category_id    TEXT NOT NULL,
  remote_id      TEXT,
  status         TEXT NOT NULL DEFAULT 'not_synced'
                 CHECK (status IN ('not_synced','queued','synced','failed','unsupported','removed')),
  last_synced_at TEXT,
  last_error     TEXT,
  updated_at     TEXT NOT NULL,
  PRIMARY KEY (account, category_id)
);
