-- Bahrain address parts (Flat, Building, Road, Block, landmark) next to the
-- one-line address every screen prints. Additive only: free-text addresses
-- keep working; the block decides the area.
ALTER TABLE customers ADD COLUMN flat TEXT;
ALTER TABLE customers ADD COLUMN building TEXT;
ALTER TABLE customers ADD COLUMN road TEXT;
ALTER TABLE customers ADD COLUMN block TEXT;
ALTER TABLE customers ADD COLUMN landmark TEXT;
ALTER TABLE delivery_orders ADD COLUMN flat TEXT;
ALTER TABLE delivery_orders ADD COLUMN building TEXT;
ALTER TABLE delivery_orders ADD COLUMN road TEXT;
ALTER TABLE delivery_orders ADD COLUMN block TEXT;
ALTER TABLE delivery_orders ADD COLUMN landmark TEXT;
CREATE INDEX ix_customers_block ON customers(block);
CREATE INDEX ix_delivery_block ON delivery_orders(block);
