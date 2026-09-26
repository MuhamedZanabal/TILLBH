-- AI workspace: photos attached in the assistant, pinned context and names
-- for conversations, scheduled briefings and their notes, WhatsApp triage.

-- A6: pinned records (JSON list of {kind, id, label}) per conversation.
ALTER TABLE ai_conversations ADD COLUMN pins_json TEXT NOT NULL DEFAULT '[]';

-- A5: a photo attached to a question (stored under data/ai-images).
CREATE TABLE ai_attachments (
  attachment_id   TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL,
  conversation_id TEXT,
  path            TEXT NOT NULL,
  media_type      TEXT NOT NULL CHECK (media_type IN ('image/png','image/jpeg','image/webp','image/gif')),
  bytes           INTEGER NOT NULL,
  sha256          TEXT NOT NULL,
  created_at      TEXT NOT NULL
);
CREATE INDEX ix_ai_attachments_user ON ai_attachments(user_id, created_at);

-- A8: a playbook that runs at a time of day while the app is open.
CREATE TABLE ai_briefings (
  briefing_id  TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  playbook     TEXT NOT NULL CHECK (playbook IN ('eod','cash_short','reorder','refund_spike')),
  at_time      TEXT NOT NULL,          -- HH:MM in the business time zone
  days         TEXT NOT NULL,          -- ISO weekdays 1-7, e.g. '1234567'
  with_ai      INTEGER NOT NULL DEFAULT 0,
  enabled      INTEGER NOT NULL DEFAULT 1,
  created_by   TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  last_run_on  TEXT                    -- business date of the last run
);

CREATE TABLE ai_notes (
  note_id      TEXT PRIMARY KEY,
  briefing_id  TEXT,
  title        TEXT NOT NULL,
  summary      TEXT,                   -- AI narration (optional), untrusted as usual
  data_json    TEXT NOT NULL,          -- the playbook steps (reads only)
  status       TEXT NOT NULL CHECK (status IN ('ok','partial','error')),
  error        TEXT,
  created_by   TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  read_at      TEXT
);
CREATE INDEX ix_ai_notes_created ON ai_notes(created_at);

-- E1: how an incoming WhatsApp message was classified.
CREATE TABLE wa_triage (
  inbox_seq    INTEGER PRIMARY KEY,
  category     TEXT NOT NULL CHECK (category IN ('order','payment','complaint','question','spam','other')),
  confidence   INTEGER NOT NULL,       -- 0-100
  source       TEXT NOT NULL CHECK (source IN ('rules','ai','person')),
  reasons_json TEXT NOT NULL,
  created_at   TEXT NOT NULL
);
