-- Not delivered. A rider can flag a drop they could not deliver (the drop
-- stays out, the board shows it as a problem); a manager then closes it as
-- not delivered, which refunds the sale (goods back on the shelf or written
-- off as damaged) and cancels the drop with that outcome. Additive only.
ALTER TABLE delivery_orders ADD COLUMN failed_note TEXT;
ALTER TABLE delivery_orders ADD COLUMN failed_at TEXT;
-- How a closed drop ended when it did not end delivered: 'not_delivered'.
ALTER TABLE delivery_orders ADD COLUMN outcome TEXT;
ALTER TABLE delivery_orders ADD COLUMN refund_id TEXT;
