//! Live progress of one AI question, for the UI to poll (`ai.stream`).
//!
//! The page picks a `stream_id`, passes it to `ai.ask`, and polls
//! `ai.stream { stream_id, after }` while the question runs. Every event the
//! assistant produces is recorded here in order: text and thinking as they
//! stream, each tool call with its input, each tool result as the model saw it,
//! nudges, provider fallbacks, and the end. Nothing is stored on disk; buffers
//! are dropped ten minutes after their last event. A buffer can only be read by
//! the user who asked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use amwapos_core::{AppError, AppResult};
use serde_json::{json, Value};

/// Sends one event (a JSON object with at least `type`).
pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;

const KEEP: Duration = Duration::from_secs(600);
const MAX_EVENTS: usize = 20_000;

struct Buf {
    user_id: String,
    events: Vec<Value>,
    done: bool,
    touched: Instant,
}

#[derive(Default)]
pub struct StreamHub {
    bufs: Mutex<HashMap<String, Buf>>,
}

/// Ids are chosen by the page: short, and letters, digits, '-' or '_' only.
pub fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl StreamHub {
    /// Start (or restart) a buffer and return the function that feeds it.
    pub fn open(self: &Arc<Self>, id: &str, user_id: &str) -> AppResult<Emit> {
        if !valid_id(id) {
            return Err(AppError::validation("Invalid stream id."));
        }
        let mut m = self.bufs.lock().map_err(|_| AppError::internal("stream store poisoned"))?;
        m.retain(|_, b| b.touched.elapsed() < KEEP);
        if let Some(b) = m.get(id) {
            if b.user_id != user_id {
                return Err(AppError::forbidden("ai.use"));
            }
        }
        m.insert(id.to_string(), Buf { user_id: user_id.to_string(), events: vec![], done: false, touched: Instant::now() });
        let hub = self.clone();
        let id = id.to_string();
        Ok(Arc::new(move |mut ev: Value| {
            if let Ok(mut m) = hub.bufs.lock() {
                if let Some(b) = m.get_mut(&id) {
                    if b.events.len() >= MAX_EVENTS {
                        return;
                    }
                    let end = matches!(ev["type"].as_str(), Some("done") | Some("error"));
                    ev["seq"] = json!(b.events.len());
                    b.events.push(ev);
                    b.touched = Instant::now();
                    b.done |= end;
                }
            }
        }))
    }

    /// Events after `after` (exclusive index), and whether the question ended.
    pub fn read(&self, id: &str, user_id: &str, after: usize) -> AppResult<Value> {
        let m = self.bufs.lock().map_err(|_| AppError::internal("stream store poisoned"))?;
        let Some(b) = m.get(id) else {
            return Ok(json!({ "events": [], "next": after, "done": false, "known": false }));
        };
        if b.user_id != user_id {
            return Err(AppError::forbidden("ai.use"));
        }
        let events: Vec<Value> = b.events.iter().skip(after).cloned().collect();
        Ok(json!({ "events": events, "next": b.events.len(), "done": b.done, "known": true }))
    }
}

/// Emit to an optional sink.
pub fn emit(sink: &Option<Emit>, ev: Value) {
    if let Some(f) = sink {
        f(ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_are_per_user_and_ordered() {
        let hub = Arc::new(StreamHub::default());
        let e = hub.open("stream-0001", "u1").unwrap();
        e(json!({ "type": "text", "delta": "a" }));
        e(json!({ "type": "done" }));
        let r = hub.read("stream-0001", "u1", 0).unwrap();
        assert_eq!(r["events"].as_array().unwrap().len(), 2);
        assert_eq!(r["done"], true);
        assert!(hub.read("stream-0001", "u2", 0).is_err());
        assert!(hub.open("stream-0001", "u2").is_err());
        assert!(hub.open("bad id!", "u1").is_err());
        assert_eq!(hub.read("stream-0001", "u1", 1).unwrap()["events"].as_array().unwrap().len(), 1);
    }
}
