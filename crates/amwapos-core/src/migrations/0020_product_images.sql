-- Product images. A product points at one stored image by content hash; the
-- image bytes live in `product_images` (normalized JPEG, base64 text so the
-- existing row sync carries them to every till). Provenance and the one-time
-- automatic lookup state are explicit, never derived from the reference.
--
-- auto_image_status:
--   not_attempted  existing products and imports (a controlled backfill queues them)
--   pending        queued for the one automatic lookup
--   processing     claimed by the lookup worker (auto_image_attempted_at = claim time)
--   found          an image was found and stored (image_source = 'automatic')
--   not_found      searched; nothing confidently usable (terminal)
--   failed         gave up after bounded retries of transient errors (terminal)
--   skipped        a person uploaded or removed the image (terminal)
ALTER TABLE products ADD COLUMN image_hash TEXT;
ALTER TABLE products ADD COLUMN image_source TEXT CHECK (image_source IN ('manual','automatic'));
ALTER TABLE products ADD COLUMN auto_image_status TEXT NOT NULL DEFAULT 'not_attempted'
  CHECK (auto_image_status IN ('not_attempted','pending','processing','found','not_found','failed','skipped'));
ALTER TABLE products ADD COLUMN auto_image_attempted_at TEXT;
ALTER TABLE products ADD COLUMN auto_image_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE products ADD COLUMN auto_image_next_at TEXT;
-- Provenance of an automatic image or the reason nothing was saved (JSON).
ALTER TABLE products ADD COLUMN auto_image_note TEXT;
CREATE INDEX ix_products_auto_image ON products(auto_image_status, auto_image_next_at);
CREATE INDEX ix_products_image_hash ON products(image_hash);

CREATE TABLE product_images (
  image_hash TEXT PRIMARY KEY,
  mime       TEXT NOT NULL CHECK (mime = 'image/jpeg'),
  width      INTEGER NOT NULL CHECK (width > 0),
  height     INTEGER NOT NULL CHECK (height > 0),
  bytes      INTEGER NOT NULL CHECK (bytes > 0),
  data_b64   TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TRIGGER trg_sync_product_images_insert AFTER INSERT ON product_images WHEN (SELECT v FROM sync_control WHERE k='suppress') IS NOT '1'
BEGIN
  INSERT INTO sync_outbox(table_name, row_pk, op, origin) VALUES ('product_images', json_object('image_hash', NEW.image_hash), 'upsert', (SELECT v FROM sync_control WHERE k='origin'));
END;
CREATE TRIGGER trg_sync_product_images_delete AFTER DELETE ON product_images WHEN (SELECT v FROM sync_control WHERE k='suppress') IS NOT '1'
BEGIN
  INSERT INTO sync_outbox(table_name, row_pk, op, origin) VALUES ('product_images', json_object('image_hash', OLD.image_hash), 'delete', (SELECT v FROM sync_control WHERE k='origin'));
END;
