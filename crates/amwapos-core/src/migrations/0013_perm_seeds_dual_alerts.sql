-- Permissions added to built-in roles by upgrades are offered once per role
-- (so a permission the owner later removes is not added back).
CREATE TABLE role_permission_seeds (
  role_id         TEXT NOT NULL,
  permission_code TEXT NOT NULL,
  seeded_at       TEXT NOT NULL,
  PRIMARY KEY (role_id, permission_code)
);

-- D1 dual control: the first of two people who confirmed a high-risk proposal.
ALTER TABLE ai_proposals ADD COLUMN first_confirmed_by TEXT;
ALTER TABLE ai_proposals ADD COLUMN first_confirmed_at TEXT;

-- B3: anomalies found by the deterministic checks, shown in the action inbox.
CREATE TABLE ai_alerts (
  alert_id     TEXT PRIMARY KEY,
  kind         TEXT NOT NULL CHECK (kind IN ('refund_spike','discount_spike','negative_stock','hub_lag','backup_overdue')),
  day_key      TEXT NOT NULL,            -- business date: one alert per kind per day
  severity     TEXT NOT NULL CHECK (severity IN ('info','warning','danger')),
  title        TEXT NOT NULL,
  detail_json  TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  dismissed_by TEXT,
  dismissed_at TEXT,
  UNIQUE (kind, day_key)
);
CREATE INDEX ix_ai_alerts_open ON ai_alerts(dismissed_at, created_at);
