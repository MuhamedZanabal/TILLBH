-- WhatsApp "order received" message kind (sent when a Send order is taken).
-- SQLite cannot change a CHECK in place, so the outbox is rebuilt with the
-- same columns and rows.
CREATE TABLE wa_outbox_new (
  message_id     TEXT PRIMARY KEY,
  operation_id   TEXT NOT NULL UNIQUE,
  payload_hash   TEXT NOT NULL DEFAULT '',
  kind           TEXT NOT NULL CHECK (kind IN ('receipt','received','dispatch','delivered','reminder','payment_ack','text','document')),
  to_phone       TEXT NOT NULL,
  customer_id    TEXT,
  sale_id        TEXT,
  delivery_id    TEXT,
  review_id      TEXT,
  lang           TEXT NOT NULL CHECK (lang IN ('en','ar')),
  body           TEXT NOT NULL,
  document_path  TEXT,
  document_name  TEXT,
  status         TEXT NOT NULL CHECK (status IN ('queued','sending','sent','failed','cancelled')),
  attempts       INTEGER NOT NULL DEFAULT 0,
  last_error     TEXT,
  wa_message_id  TEXT,
  created_by     TEXT NOT NULL,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL,
  sent_at        TEXT,
  next_attempt_at TEXT
);
INSERT INTO wa_outbox_new(message_id, operation_id, payload_hash, kind, to_phone, customer_id, sale_id, delivery_id, review_id, lang, body,
    document_path, document_name, status, attempts, last_error, wa_message_id, created_by, created_at, updated_at, sent_at, next_attempt_at)
  SELECT message_id, operation_id, payload_hash, kind, to_phone, customer_id, sale_id, delivery_id, review_id, lang, body,
    document_path, document_name, status, attempts, last_error, wa_message_id, created_by, created_at, updated_at, sent_at, next_attempt_at
  FROM wa_outbox;
DROP TABLE wa_outbox;
ALTER TABLE wa_outbox_new RENAME TO wa_outbox;
CREATE INDEX ix_wa_outbox_status ON wa_outbox(status, next_attempt_at);
CREATE INDEX ix_wa_outbox_sale ON wa_outbox(sale_id);
