-- TILLBH Shop Server: independent PostgreSQL 15+ authority foundation.
-- Additive schema only. Existing terminal SQLite databases are not modified.
-- Execution and restore must be tested before any production deployment.
BEGIN;
CREATE SCHEMA IF NOT EXISTS tillbh_authority;
SET LOCAL search_path TO tillbh_authority, public;

CREATE TABLE tenants (
  tenant_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  business_name text NOT NULL CHECK (length(trim(business_name)) > 0),
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE branches (
  branch_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL REFERENCES tenants(tenant_id),
  name text NOT NULL CHECK (length(trim(name)) > 0),
  UNIQUE (tenant_id, branch_id)
);

CREATE TABLE terminal_devices (
  terminal_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL,
  branch_id uuid NOT NULL,
  approved boolean NOT NULL DEFAULT false,
  revoked_at timestamptz,
  key_fingerprint text NOT NULL,
  UNIQUE (tenant_id, terminal_id),
  FOREIGN KEY (tenant_id, branch_id) REFERENCES branches(tenant_id, branch_id),
  CHECK (NOT (approved AND revoked_at IS NOT NULL))
);

CREATE TABLE products (
  product_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL REFERENCES tenants(tenant_id),
  sku text NOT NULL,
  name text NOT NULL CHECK (length(trim(name)) > 0),
  active boolean NOT NULL DEFAULT true,
  UNIQUE (tenant_id, product_id),
  UNIQUE (tenant_id, sku)
);

-- Historical checkout calculations use immutable sale-line price/tax snapshots.
-- Prices and policies must be versioned separately before enabling server checkout.
CREATE TABLE sales (
  sale_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL,
  branch_id uuid NOT NULL,
  terminal_id uuid NOT NULL,
  operation_id uuid NOT NULL,
  payload_sha256 text NOT NULL CHECK (payload_sha256 ~ '^[0-9a-f]{64}$'),
  receipt_number text NOT NULL,
  customer_id uuid,
  gross_minor bigint NOT NULL CHECK (gross_minor >= 0),
  discount_minor bigint NOT NULL CHECK (discount_minor >= 0),
  net_minor bigint NOT NULL CHECK (net_minor >= 0),
  vat_minor bigint NOT NULL CHECK (vat_minor >= 0),
  total_minor bigint NOT NULL CHECK (total_minor >= 0),
  receivable_minor bigint NOT NULL DEFAULT 0 CHECK (receivable_minor >= 0),
  source_type text NOT NULL DEFAULT 'online' CHECK (source_type IN ('online', 'offline_replayed')),
  committed_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, sale_id),
  UNIQUE (tenant_id, terminal_id, operation_id),
  UNIQUE (tenant_id, branch_id, receipt_number),
  FOREIGN KEY (tenant_id, terminal_id) REFERENCES terminal_devices(tenant_id, terminal_id),
  FOREIGN KEY (tenant_id, branch_id) REFERENCES branches(tenant_id, branch_id),
  CHECK (gross_minor - discount_minor = net_minor),
  CHECK (net_minor + vat_minor = total_minor),
  CHECK (receivable_minor <= total_minor),
  CHECK (receivable_minor = 0 OR customer_id IS NOT NULL)
);

CREATE TABLE sale_lines (
  sale_line_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL,
  sale_id uuid NOT NULL,
  product_id uuid NOT NULL,
  product_name_snapshot text NOT NULL,
  qty_milli bigint NOT NULL CHECK (qty_milli > 0),
  unit_price_minor bigint NOT NULL CHECK (unit_price_minor >= 0),
  gross_minor bigint NOT NULL CHECK (gross_minor >= 0),
  discount_minor bigint NOT NULL CHECK (discount_minor >= 0),
  net_minor bigint NOT NULL CHECK (net_minor >= 0),
  vat_minor bigint NOT NULL CHECK (vat_minor >= 0),
  line_total_minor bigint NOT NULL CHECK (line_total_minor >= 0),
  tax_policy_version text NOT NULL,
  UNIQUE (tenant_id, sale_line_id),
  FOREIGN KEY (tenant_id, sale_id) REFERENCES sales(tenant_id, sale_id),
  FOREIGN KEY (tenant_id, product_id) REFERENCES products(tenant_id, product_id),
  CHECK (gross_minor - discount_minor = net_minor),
  CHECK (net_minor + vat_minor = line_total_minor)
);

-- Only *accepted/verified* tender amounts belong in this settlement ledger.
-- Pending screenshots, intentions, and OCR guesses must live in a separate queue.
CREATE TABLE sale_tenders (
  tender_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL,
  sale_id uuid NOT NULL,
  method text NOT NULL CHECK (method IN ('cash','card','benefitpay','bank_transfer')),
  applied_minor bigint NOT NULL CHECK (applied_minor > 0),
  tendered_minor bigint,
  change_minor bigint,
  verification_kind text NOT NULL CHECK (verification_kind IN ('cash_counter','provider_authoritative','manual_approval')),
  provider_reference text,
  verifier_user_id uuid,
  verified_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (tenant_id, sale_id) REFERENCES sales(tenant_id, sale_id),
  CHECK (
    (method = 'cash'
      AND verification_kind = 'cash_counter'
      AND tendered_minor IS NOT NULL AND change_minor IS NOT NULL
      AND tendered_minor >= applied_minor AND change_minor >= 0
      AND tendered_minor - change_minor = applied_minor)
    OR
    (method <> 'cash' AND tendered_minor IS NULL AND change_minor IS NULL
      AND ((verification_kind = 'provider_authoritative' AND provider_reference IS NOT NULL)
        OR (verification_kind = 'manual_approval' AND verifier_user_id IS NOT NULL)))
  )
);

CREATE TABLE stock_movements (
  movement_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL,
  branch_id uuid NOT NULL,
  product_id uuid NOT NULL,
  sale_line_id uuid,
  movement_kind text NOT NULL CHECK (movement_kind IN ('sale','refund','receive','adjust','transfer_in','transfer_out')),
  qty_delta_milli bigint NOT NULL CHECK (qty_delta_milli <> 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (tenant_id, branch_id) REFERENCES branches(tenant_id, branch_id),
  FOREIGN KEY (tenant_id, product_id) REFERENCES products(tenant_id, product_id),
  FOREIGN KEY (tenant_id, sale_line_id) REFERENCES sale_lines(tenant_id, sale_line_id),
  CHECK ((movement_kind = 'sale' AND qty_delta_milli < 0 AND sale_line_id IS NOT NULL)
    OR (movement_kind <> 'sale' AND sale_line_id IS NULL))
);
CREATE UNIQUE INDEX stock_sale_line_once ON stock_movements(tenant_id, sale_line_id)
  WHERE movement_kind = 'sale';

CREATE TABLE gl_accounts (
  account_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL REFERENCES tenants(tenant_id),
  code text NOT NULL,
  name text NOT NULL,
  UNIQUE (tenant_id, account_id),
  UNIQUE (tenant_id, code)
);
CREATE TABLE gl_journals (
  journal_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL REFERENCES tenants(tenant_id),
  source_sale_id uuid,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, journal_id),
  UNIQUE (tenant_id, source_sale_id),
  FOREIGN KEY (tenant_id, source_sale_id) REFERENCES sales(tenant_id, sale_id)
);
CREATE TABLE gl_entries (
  entry_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id uuid NOT NULL,
  journal_id uuid NOT NULL,
  account_id uuid NOT NULL,
  debit_minor bigint NOT NULL DEFAULT 0 CHECK (debit_minor >= 0),
  credit_minor bigint NOT NULL DEFAULT 0 CHECK (credit_minor >= 0),
  FOREIGN KEY (tenant_id, journal_id) REFERENCES gl_journals(tenant_id, journal_id),
  FOREIGN KEY (tenant_id, account_id) REFERENCES gl_accounts(tenant_id, account_id),
  CHECK ((debit_minor > 0 AND credit_minor = 0) OR (credit_minor > 0 AND debit_minor = 0))
);

CREATE TABLE server_outbox (
  event_sequence bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  tenant_id uuid NOT NULL REFERENCES tenants(tenant_id),
  event_id uuid NOT NULL DEFAULT gen_random_uuid(),
  event_kind text NOT NULL,
  aggregate_id uuid NOT NULL,
  payload jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, event_id)
);

-- Server must reject all UPDATE/DELETE attempts on committed financial history.
CREATE FUNCTION forbid_financial_mutation() RETURNS trigger LANGUAGE plpgsql AS $fn$
BEGIN
  RAISE EXCEPTION 'immutable financial record: %', TG_TABLE_NAME USING ERRCODE = '55000';
END;
$fn$;
CREATE TRIGGER sales_append_only BEFORE UPDATE OR DELETE ON sales FOR EACH ROW EXECUTE FUNCTION forbid_financial_mutation();
CREATE TRIGGER sale_lines_append_only BEFORE UPDATE OR DELETE ON sale_lines FOR EACH ROW EXECUTE FUNCTION forbid_financial_mutation();
CREATE TRIGGER sale_tenders_append_only BEFORE UPDATE OR DELETE ON sale_tenders FOR EACH ROW EXECUTE FUNCTION forbid_financial_mutation();
CREATE TRIGGER stock_movements_append_only BEFORE UPDATE OR DELETE ON stock_movements FOR EACH ROW EXECUTE FUNCTION forbid_financial_mutation();
CREATE TRIGGER gl_journals_append_only BEFORE UPDATE OR DELETE ON gl_journals FOR EACH ROW EXECUTE FUNCTION forbid_financial_mutation();
CREATE TRIGGER gl_entries_append_only BEFORE UPDATE OR DELETE ON gl_entries FOR EACH ROW EXECUTE FUNCTION forbid_financial_mutation();

-- A sale's snapshot, tenders, and stock movements must agree at COMMIT.
CREATE FUNCTION assert_sale_consistent() RETURNS trigger LANGUAGE plpgsql AS $fn$
DECLARE
  values_total record;
  tender_total bigint;
  sale_stock_count bigint;
  sale_line_count bigint;
BEGIN
  SELECT
    coalesce(sum(gross_minor), 0) AS gross,
    coalesce(sum(discount_minor), 0) AS discount,
    coalesce(sum(net_minor), 0) AS net,
    coalesce(sum(vat_minor), 0) AS vat,
    coalesce(sum(line_total_minor), 0) AS total,
    count(*) AS lines
    INTO values_total FROM sale_lines WHERE tenant_id = NEW.tenant_id AND sale_id = NEW.sale_id;
  SELECT coalesce(sum(applied_minor), 0) INTO tender_total
    FROM sale_tenders WHERE tenant_id = NEW.tenant_id AND sale_id = NEW.sale_id;
  SELECT count(*) INTO sale_line_count FROM sale_lines
    WHERE tenant_id = NEW.tenant_id AND sale_id = NEW.sale_id;
  SELECT count(*) INTO sale_stock_count FROM stock_movements sm
    JOIN sale_lines sl ON sl.tenant_id = sm.tenant_id AND sl.sale_line_id = sm.sale_line_id
    WHERE sl.tenant_id = NEW.tenant_id AND sl.sale_id = NEW.sale_id AND sm.movement_kind = 'sale';
  IF values_total.lines = 0
     OR values_total.gross <> NEW.gross_minor OR values_total.discount <> NEW.discount_minor
     OR values_total.net <> NEW.net_minor OR values_total.vat <> NEW.vat_minor
     OR values_total.total <> NEW.total_minor
     OR tender_total + NEW.receivable_minor <> NEW.total_minor
     OR sale_stock_count <> sale_line_count
  THEN
    RAISE EXCEPTION 'sale snapshot/settlement/inventory does not reconcile: %', NEW.sale_id
      USING ERRCODE = '23514';
  END IF;
  RETURN NULL;
END;
$fn$;
CREATE CONSTRAINT TRIGGER sale_commit_invariant
  AFTER INSERT ON sales DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION assert_sale_consistent();

-- A journal must be balanced when the transaction commits.
CREATE FUNCTION assert_journal_balanced() RETURNS trigger LANGUAGE plpgsql AS $fn$
DECLARE debits bigint; credits bigint;
BEGIN
  SELECT coalesce(sum(debit_minor), 0), coalesce(sum(credit_minor), 0)
    INTO debits, credits FROM gl_entries
    WHERE tenant_id = NEW.tenant_id AND journal_id = NEW.journal_id;
  IF debits = 0 OR debits <> credits THEN
    RAISE EXCEPTION 'unbalanced journal: %', NEW.journal_id USING ERRCODE = '23514';
  END IF;
  RETURN NULL;
END;
$fn$;
CREATE CONSTRAINT TRIGGER journal_commit_invariant
  AFTER INSERT ON gl_journals DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION assert_journal_balanced();

-- Application sessions MUST set tillbh.tenant_id to an authorized tenant UUID
-- inside every DB transaction. No setting means no tenant rows are accessible.
DO $rls$
DECLARE table_name text;
BEGIN
  FOREACH table_name IN ARRAY ARRAY[
    'tenants','branches','terminal_devices','products','sales','sale_lines',
    'sale_tenders','stock_movements','gl_accounts','gl_journals','gl_entries','server_outbox'
  ] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', table_name);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', table_name);
    EXECUTE format(
      'CREATE POLICY tenant_scope ON %I FOR ALL USING (%I = nullif(current_setting(''tillbh.tenant_id'', true), '''')::uuid) WITH CHECK (%I = nullif(current_setting(''tillbh.tenant_id'', true), '''')::uuid)',
      table_name,
      CASE WHEN table_name = 'tenants' THEN 'tenant_id' ELSE 'tenant_id' END,
      'tenant_id'
    );
  END LOOP;
END;
$rls$;
COMMIT;
