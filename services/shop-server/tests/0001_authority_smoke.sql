-- Run only against an empty disposable PostgreSQL 15+ test database:
--   psql -v ON_ERROR_STOP=1 -f services/shop-server/migrations/0001_authority_core.sql
--   psql -v ON_ERROR_STOP=1 -f services/shop-server/tests/0001_authority_smoke.sql
-- This is a transactional smoke fixture and intentionally rolls back its data.
\set ON_ERROR_STOP on
BEGIN;
SET LOCAL search_path = tillbh_authority, public;
SET LOCAL tillbh.tenant_id = '11111111-1111-4111-8111-111111111111';

INSERT INTO tenants(tenant_id, business_name)
VALUES ('11111111-1111-4111-8111-111111111111', 'Fixture Supermarket');
INSERT INTO branches(tenant_id, branch_id, name)
VALUES ('11111111-1111-4111-8111-111111111111', '22222222-2222-4222-8222-222222222222', 'Main');
INSERT INTO terminal_devices(tenant_id, branch_id, terminal_id, approved, key_fingerprint)
VALUES ('11111111-1111-4111-8111-111111111111', '22222222-2222-4222-8222-222222222222',
        '33333333-3333-4333-8333-333333333333', true, 'test-fingerprint-not-a-key');
INSERT INTO products(tenant_id, product_id, sku, name)
VALUES ('11111111-1111-4111-8111-111111111111', '44444444-4444-4444-8444-444444444444', 'F001', 'Fixture Product');

-- The exact required finance sample: 4.488 - 0.150 + 0.434 = 4.772 BHD.
INSERT INTO sales(sale_id,tenant_id,branch_id,terminal_id,operation_id,payload_sha256,
                  receipt_number,gross_minor,discount_minor,net_minor,vat_minor,total_minor,receivable_minor)
VALUES ('55555555-5555-4555-8555-555555555555', '11111111-1111-4111-8111-111111111111',
        '22222222-2222-4222-8222-222222222222','33333333-3333-4333-8333-333333333333',
        '66666666-6666-4666-8666-666666666666',
        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        'T01-0000001',4488,150,4338,434,4772,0);
INSERT INTO sale_lines(sale_line_id,tenant_id,sale_id,product_id,product_name_snapshot,qty_milli,
                       unit_price_minor,gross_minor,discount_minor,net_minor,vat_minor,line_total_minor,tax_policy_version)
VALUES ('77777777-7777-4777-8777-777777777777','11111111-1111-4111-8111-111111111111',
        '55555555-5555-4555-8555-555555555555','44444444-4444-4444-8444-444444444444',
        'Fixture Product',1000,4488,4488,150,4338,434,4772,'bh-vat10-tax-exclusive-v1');
INSERT INTO sale_tenders(tenant_id,sale_id,method,applied_minor,tendered_minor,change_minor,verification_kind)
VALUES ('11111111-1111-4111-8111-111111111111','55555555-5555-4555-8555-555555555555',
        'cash',4772,5000,228,'cash_counter');
INSERT INTO stock_movements(tenant_id,branch_id,product_id,sale_line_id,movement_kind,qty_delta_milli)
VALUES ('11111111-1111-4111-8111-111111111111','22222222-2222-4222-8222-222222222222',
        '44444444-4444-4444-8444-444444444444','77777777-7777-4777-8777-777777777777',
        'sale',-1000);

INSERT INTO gl_accounts(tenant_id,account_id,code,name) VALUES
('11111111-1111-4111-8111-111111111111','88888888-8888-4888-8888-888888888888','1000','Cash'),
('11111111-1111-4111-8111-111111111111','99999999-9999-4999-8999-999999999999','4000','Sales'),
('11111111-1111-4111-8111-111111111111','aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','2200','VAT');
INSERT INTO gl_journals(tenant_id,journal_id,source_sale_id)
VALUES ('11111111-1111-4111-8111-111111111111','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
        '55555555-5555-4555-8555-555555555555');
INSERT INTO gl_entries(tenant_id,journal_id,account_id,debit_minor,credit_minor) VALUES
('11111111-1111-4111-8111-111111111111','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','88888888-8888-4888-8888-888888888888',4772,0),
('11111111-1111-4111-8111-111111111111','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','99999999-9999-4999-8999-999999999999',0,4338),
('11111111-1111-4111-8111-111111111111','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',0,434);

-- Force deferred finance checks before inspection, rather than waiting for ROLLBACK.
SET CONSTRAINTS ALL IMMEDIATE;

DO $test$
DECLARE matches bigint;
BEGIN
  SELECT count(*) INTO matches FROM sales
    WHERE total_minor=4772 AND net_minor=4338 AND vat_minor=434
      AND tenant_id='11111111-1111-4111-8111-111111111111';
  IF matches<>1 THEN RAISE EXCEPTION 'exact 4.772 BHD fixture not found'; END IF;

  BEGIN
    INSERT INTO sales(sale_id,tenant_id,branch_id,terminal_id,operation_id,payload_sha256,
      receipt_number,gross_minor,discount_minor,net_minor,vat_minor,total_minor)
    VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc',
      '11111111-1111-4111-8111-111111111111',
      '22222222-2222-4222-8222-222222222222',
      '33333333-3333-4333-8333-333333333333',
      '66666666-6666-4666-8666-666666666666',
      'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
      'T01-0000002',100,0,100,0,100);
    RAISE EXCEPTION 'duplicate operation was accepted';
  EXCEPTION WHEN unique_violation THEN NULL;
  END;

  BEGIN
    UPDATE sales SET total_minor=1 WHERE sale_id='55555555-5555-4555-8555-555555555555';
    RAISE EXCEPTION 'immutable sale was modified';
  EXCEPTION WHEN SQLSTATE '55000' THEN NULL;
  END;
END;
$test$;

-- A different tenant must not read the first tenant's sales.
SET LOCAL tillbh.tenant_id = 'dddddddd-dddd-4ddd-8ddd-dddddddddddd';
DO $test$
BEGIN
  IF (SELECT count(*) FROM sales) <> 0 THEN
    RAISE EXCEPTION 'RLS tenant-isolation violation';
  END IF;
END;
$test$;
ROLLBACK;
