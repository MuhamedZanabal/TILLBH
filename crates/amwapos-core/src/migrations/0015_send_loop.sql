-- One operating loop: customer → channel → ticket (sale or digital order) →
-- drop (delivery) → close. Additive only: existing delivery rows stay open
-- and editable.

-- The drop knows its ticket, branch, channel and one payment state used
-- everywhere: unpaid | recorded | screenshot_pending | paid.
ALTER TABLE delivery_orders ADD COLUMN order_id TEXT;
ALTER TABLE delivery_orders ADD COLUMN branch_id TEXT;
ALTER TABLE delivery_orders ADD COLUMN channel TEXT;
ALTER TABLE delivery_orders ADD COLUMN pay_state TEXT;
UPDATE delivery_orders SET pay_state = CASE payment_status WHEN 'paid' THEN 'paid' ELSE 'unpaid' END WHERE pay_state IS NULL;
UPDATE delivery_orders SET branch_id = (SELECT branch_id FROM sales WHERE sales.sale_id = delivery_orders.sale_id)
  WHERE branch_id IS NULL AND sale_id IS NOT NULL;
UPDATE delivery_orders SET order_id = (SELECT order_id FROM digital_orders o WHERE o.delivery_id = delivery_orders.delivery_id)
  WHERE order_id IS NULL;
CREATE INDEX ix_delivery_sale ON delivery_orders(sale_id);
CREATE INDEX ix_delivery_order ON delivery_orders(order_id);

-- Money collected for a pay-on-delivery ticket after the sale (at the door or
-- later at the till). An immutable record like a payment; cash counts in the
-- drawer of the shift where it was collected.
CREATE TABLE sale_collections (
  collection_id TEXT PRIMARY KEY,
  sale_id       TEXT,
  delivery_id   TEXT,
  method        TEXT NOT NULL,
  amount_minor  INTEGER NOT NULL CHECK (amount_minor > 0),
  reference     TEXT,
  shift_id      TEXT,
  branch_id     TEXT NOT NULL,
  device_id     TEXT NOT NULL,
  user_id       TEXT NOT NULL,
  operation_id  TEXT NOT NULL UNIQUE,
  created_at    TEXT NOT NULL
);
CREATE INDEX ix_collections_sale ON sale_collections(sale_id);
CREATE INDEX ix_collections_delivery ON sale_collections(delivery_id);
CREATE INDEX ix_collections_shift ON sale_collections(shift_id);
CREATE TRIGGER trg_sync_sale_collections_insert AFTER INSERT ON sale_collections WHEN (SELECT v FROM sync_control WHERE k='suppress') IS NOT '1'
BEGIN
  INSERT INTO sync_outbox(table_name, row_pk, op, origin) VALUES ('sale_collections', json_object('collection_id', NEW.collection_id), 'upsert', (SELECT v FROM sync_control WHERE k='origin'));
END;

-- A WhatsApp chat a person linked to a customer by hand (a number match needs
-- no row). Local to the computer that runs WhatsApp.
CREATE TABLE wa_chat_links (
  chat        TEXT PRIMARY KEY,
  customer_id TEXT NOT NULL,
  linked_by   TEXT NOT NULL,
  linked_at   TEXT NOT NULL
);
