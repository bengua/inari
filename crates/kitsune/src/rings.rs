//! Bringing a note back: the time reminders that fall due, the place reminders the phone says it
//! entered, and the local webhook (n8n, Home Assistant…) told when one rings. The core's scheduler
//! calls [`Kitsune::tick`]; nothing here polls a position — the phone's own geofencing reports
//! the entry.

use crate::brain::{Kitsune, Retried};
use crate::net;
use crate::store::Note;
use crate::{clock, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

/// How reminders ring, from the GUI.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RingConfig {
    /// How often the core looks for due reminders, in seconds.
    pub tick_seconds: u64,
    /// How often what waited for a server is tried again, in seconds (0: never on its own).
    pub retry_seconds: u64,
    /// A place reminder rings again on a new entry only after this many minutes.
    pub geofence_quiet_minutes: i64,
    /// Told (POST, JSON) when a reminder rings: n8n, Home Assistant… on the private network
    /// (empty: nobody else is told).
    pub webhook_url: String,
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            tick_seconds: 30,
            retry_seconds: 300,
            geofence_quiet_minutes: 30,
            webhook_url: String::new(),
        }
    }
}

/// What one tick did.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Tick {
    /// The time reminders that rang.
    pub rang: Vec<Note>,
    /// What was tried again, when it was time to.
    pub retried: Option<Retried>,
}

impl Kitsune {
    /// One beat of the scheduler at `now`: the time reminders due ring (once each), and every
    /// `retry_seconds` what waited for a server is tried again (`last_retry` is when it last was).
    pub fn tick(&self, now: i64, last_retry: i64) -> Result<Tick> {
        let rang = self.store().due(now)?;
        let every = self.settings().rings.retry_seconds as i64;
        let retried = if every > 0 && now - last_retry >= every {
            Some(self.retry()?)
        } else {
            None
        };
        Ok(Tick { rang, retried })
    }

    /// The phone's geofencing says the person entered around (lat, lon): the active place
    /// reminders there ring, unless they rang within the quiet minutes.
    pub fn entered(&self, lat: f64, lon: f64) -> Result<Vec<Note>> {
        self.seen_at(lat, lon);
        let now = clock::now();
        let quiet = self.settings().rings.geofence_quiet_minutes.max(0) * 60;
        let mut rang = Vec::new();
        for n in self.store().entered(lat, lon)? {
            let recent = n
                .notified_at
                .as_deref()
                .and_then(clock::parse)
                .is_some_and(|at| now - at < quiet);
            if !recent {
                self.store().mark_notified(n.id, now)?;
                rang.push(self.store().get_note(n.id)?.expect("still there"));
            }
        }
        Ok(rang)
    }
}

/// Tell the webhook that reminders rang (best effort: a webhook away is not an error for the
/// person, whose page and phone already rang). Private network only, as everything Kitsune sends.
pub fn tell_webhook(cfg: &RingConfig, rang: &[Note]) -> Result<()> {
    if cfg.webhook_url.trim().is_empty() {
        return Ok(());
    }
    for n in rang {
        net::post_json(
            cfg.webhook_url.trim(),
            &json!({"event": "kitsune.reminder", "note": n}),
            &[],
            Duration::from_secs(5),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Reminder;
    use crate::Settings;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_due_reminder_rings_once_and_tells_the_webhook() {
        let dir = tempfile::tempdir().unwrap();
        let k = Kitsune::open(dir.path()).unwrap();
        let id = k.capture_text("Appeler le comptable", vec![]).unwrap();
        k.store()
            .set_reminder(
                id,
                &Reminder::Time {
                    due_date: "2026-10-31T09:00:00Z".into(),
                },
            )
            .unwrap();
        let before = clock::parse("2026-10-31T08:59:00Z").unwrap();
        let at = clock::parse("2026-10-31T09:00:30Z").unwrap();
        assert!(k.tick(before, before).unwrap().rang.is_empty());
        let t = k.tick(at, at).unwrap();
        assert_eq!(t.rang.len(), 1);
        assert_eq!(t.rang[0].id, id);
        assert!(t.retried.is_none(), "not yet time to retry");
        assert!(k.tick(at + 60, at).unwrap().rang.is_empty(), "rings once");
        assert!(
            k.tick(at + 400, at).unwrap().retried.is_some(),
            "retried after retry_seconds"
        );

        // the webhook hears it
        let seen: Arc<Mutex<String>> = Arc::default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let s2 = Arc::clone(&seen);
        std::thread::spawn(move || {
            if let Some(Ok(mut s)) = listener.incoming().next() {
                let mut buf = vec![0u8; 65536];
                let mut got = Vec::new();
                while !String::from_utf8_lossy(&got).contains("comptable") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
                *s2.lock().unwrap() = String::from_utf8_lossy(&got).into_owned();
                let _ = s.write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let cfg = RingConfig {
            webhook_url: format!("http://127.0.0.1:{port}/webhook/kitsune"),
            ..Default::default()
        };
        tell_webhook(&cfg, &t.rang).unwrap();
        let got = seen.lock().unwrap().clone();
        assert!(got.starts_with("POST /webhook/kitsune"), "{got}");
        assert!(
            got.contains("\"event\":\"kitsune.reminder\"") && got.contains("Appeler le comptable")
        );
        // a cloud webhook is refused like any other cloud address
        let cloud = RingConfig {
            webhook_url: "http://8.8.8.8/hook".into(),
            ..Default::default()
        };
        assert!(tell_webhook(&cloud, &t.rang).is_err());
    }

    #[test]
    fn entering_a_place_rings_its_reminders_then_stays_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let mut k = Kitsune::open(dir.path()).unwrap();
        k.set_settings(Settings::default()).unwrap();
        let id = k.capture_text("Donner le code à Julie", vec![]).unwrap();
        k.store()
            .set_reminder(
                id,
                &Reminder::Geofence {
                    latitude: 48.8650,
                    longitude: 2.3780,
                    radius_meters: 100,
                },
            )
            .unwrap();
        assert!(
            k.entered(48.8704, 2.3780).unwrap().is_empty(),
            "~600 m away"
        );
        let rang = k.entered(48.8654, 2.3780).unwrap();
        assert_eq!(rang.len(), 1);
        assert!(rang[0].notified_at.is_some());
        assert!(
            k.entered(48.8654, 2.3780).unwrap().is_empty(),
            "quiet right after"
        );
        // dismissed: never again
        k.store().dismiss(id).unwrap();
        let mut s = k.settings().clone();
        s.rings.geofence_quiet_minutes = 0;
        k.set_settings(s).unwrap();
        assert!(k.entered(48.8654, 2.3780).unwrap().is_empty());
    }
}
