-- Contacts saved on the linked WhatsApp phone (the phone's address-book name,
-- not the person's own WhatsApp name). Filled from WhatsApp's contact sync on
-- this computer only; never synced to other tills. Import copies them into
-- customers (which do sync).
CREATE TABLE wa_contacts (
  jid          TEXT PRIMARY KEY,
  phone        TEXT,
  full_name    TEXT,
  first_name   TEXT,
  updated_at   TEXT NOT NULL,
  imported_customer_id TEXT,
  imported_at  TEXT
);
CREATE INDEX ix_wa_contacts_phone ON wa_contacts(phone);
