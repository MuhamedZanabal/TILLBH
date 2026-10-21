-- Guard against appending sale children after the checkout transaction completed.
-- A successful sale may create its lines, accepted tenders, and stock movements
-- ONLY in the same database transaction as the sale header.
BEGIN;
SET LOCAL search_path TO tillbh_authority, public;

ALTER TABLE sales
  ADD COLUMN origin_xid bigint NOT NULL DEFAULT txid_current();

-- Derive the ID inside the database: application input may not spoof origin_xid.
CREATE FUNCTION stamp_sale_origin_xid() RETURNS trigger LANGUAGE plpgsql SET search_path = tillbh_authority, pg_catalog AS $fn$
BEGIN
  NEW.origin_xid := txid_current();
  RETURN NEW;
END;
$fn$;
CREATE TRIGGER sale_origin_xid BEFORE INSERT ON sales
  FOR EACH ROW EXECUTE FUNCTION stamp_sale_origin_xid();

CREATE FUNCTION enforce_sale_child_origin() RETURNS trigger LANGUAGE plpgsql SET search_path = tillbh_authority, pg_catalog AS $fn$
DECLARE parent_sale_id uuid;
BEGIN
  IF TG_TABLE_NAME = 'stock_movements' THEN
    IF NEW.movement_kind <> 'sale' THEN RETURN NEW; END IF;
    SELECT sl.sale_id INTO parent_sale_id FROM sale_lines sl
      WHERE sl.tenant_id = NEW.tenant_id AND sl.sale_line_id = NEW.sale_line_id;
  ELSE
    parent_sale_id := NEW.sale_id;
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM sales s WHERE s.tenant_id = NEW.tenant_id
      AND s.sale_id = parent_sale_id AND s.origin_xid = txid_current()
  ) THEN
    RAISE EXCEPTION 'sale children must commit atomically with the sale header'
      USING ERRCODE = '23514';
  END IF;
  RETURN NEW;
END;
$fn$;

CREATE TRIGGER sale_lines_same_transaction BEFORE INSERT ON sale_lines
  FOR EACH ROW EXECUTE FUNCTION enforce_sale_child_origin();
CREATE TRIGGER sale_tenders_same_transaction BEFORE INSERT ON sale_tenders
  FOR EACH ROW EXECUTE FUNCTION enforce_sale_child_origin();
CREATE TRIGGER sale_stock_same_transaction BEFORE INSERT ON stock_movements
  FOR EACH ROW EXECUTE FUNCTION enforce_sale_child_origin();

ALTER TABLE terminal_devices
  ADD CONSTRAINT terminal_tenant_branch_identity UNIQUE (tenant_id, branch_id, terminal_id);
ALTER TABLE sales
  ADD CONSTRAINT sale_terminal_same_branch
  FOREIGN KEY (tenant_id, branch_id, terminal_id)
  REFERENCES terminal_devices (tenant_id, branch_id, terminal_id);

COMMIT;
