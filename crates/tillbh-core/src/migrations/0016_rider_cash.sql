-- Rider cash custody. Cash a rider takes at the door is recorded against the
-- ticket at once (the customer has paid) but sits with the rider, in no
-- drawer, until a cashier counts it in at a hand-over. Additive only.

-- Who holds cash collected outside a shift (the rider). NULL for money taken
-- at a till, which is in that shift's drawer already.
ALTER TABLE sale_collections ADD COLUMN held_by TEXT;
CREATE INDEX ix_collections_held ON sale_collections(held_by);

-- A rider hands their cash to a cashier: what the records say they hold
-- (expected), what was counted into the drawer, and the difference.
CREATE TABLE rider_handovers (
  handover_id    TEXT PRIMARY KEY,
  handover_number TEXT NOT NULL UNIQUE,
  rider_user_id  TEXT NOT NULL,
  expected_minor INTEGER NOT NULL CHECK (expected_minor >= 0),
  counted_minor  INTEGER NOT NULL CHECK (counted_minor >= 0),
  variance_minor INTEGER NOT NULL,
  note           TEXT,
  shift_id       TEXT NOT NULL,
  branch_id      TEXT NOT NULL,
  device_id      TEXT NOT NULL,
  user_id        TEXT NOT NULL,
  operation_id   TEXT NOT NULL UNIQUE,
  created_at     TEXT NOT NULL
);
CREATE INDEX ix_handovers_shift ON rider_handovers(shift_id);
CREATE INDEX ix_handovers_rider ON rider_handovers(rider_user_id);

-- Which collections a hand-over covered. A collection is handed over once.
CREATE TABLE rider_handover_items (
  collection_id TEXT PRIMARY KEY,
  handover_id   TEXT NOT NULL REFERENCES rider_handovers(handover_id),
  amount_minor  INTEGER NOT NULL
);
CREATE INDEX ix_handover_items_handover ON rider_handover_items(handover_id);

CREATE TRIGGER trg_sync_rider_handovers_insert AFTER INSERT ON rider_handovers WHEN (SELECT v FROM sync_control WHERE k='suppress') IS NOT '1'
BEGIN
  INSERT INTO sync_outbox(table_name, row_pk, op, origin) VALUES ('rider_handovers', json_object('handover_id', NEW.handover_id), 'upsert', (SELECT v FROM sync_control WHERE k='origin'));
END;
CREATE TRIGGER trg_sync_rider_handover_items_insert AFTER INSERT ON rider_handover_items WHEN (SELECT v FROM sync_control WHERE k='suppress') IS NOT '1'
BEGIN
  INSERT INTO sync_outbox(table_name, row_pk, op, origin) VALUES ('rider_handover_items', json_object('collection_id', NEW.collection_id), 'upsert', (SELECT v FROM sync_control WHERE k='origin'));
END;
