//! What brings a note back, evaluated here and marked in the store: a time reached (once, or
//! every so often), a place entered or left, an event of the system. The host drives these — it
//! ticks the clock, reports a new position, names an event — and Kitsune says which triggers fire
//! and keeps their state. The watcher of the clock and of the position lives on the host (the
//! phone's geofence, the core's scheduler), never here: this is only the rule.

use crate::search::haversine_m;
use crate::store::{now, Trigger};
use crate::{Result, Store};
use rusqlite::params;
use serde_json::Value;

/// A time trigger's details.
fn time_spec(t: &Trigger) -> (Option<f64>, Option<f64>) {
    let at = t.spec.get("at").and_then(Value::as_f64);
    // "every" seconds: a recurring trigger; absent or <= 0 means once
    let every = t
        .spec
        .get("every")
        .and_then(Value::as_f64)
        .filter(|e| *e > 0.0);
    (at, every)
}

/// A geo trigger's circle and which way it fires.
fn geo_spec(t: &Trigger) -> Option<(f64, f64, f64, bool)> {
    let lat = t.spec.get("lat").and_then(Value::as_f64)?;
    let lon = t.spec.get("lon").and_then(Value::as_f64)?;
    let radius = t
        .spec
        .get("radius_m")
        .and_then(Value::as_f64)
        .unwrap_or(150.0);
    // on: "enter" (default) or "exit"
    let on_enter = t.spec.get("on").and_then(Value::as_str) != Some("exit");
    Some((lat, lon, radius, on_enter))
}

impl Store {
    /// The time triggers due at or before `at`: each is marked fired; a recurring one is moved
    /// to its next time after `at`, a one-shot is switched off.
    pub fn due_time(&self, at: f64) -> Result<Vec<Trigger>> {
        let due: Vec<Trigger> = self
            .list_triggers()?
            .into_iter()
            .filter(|t| t.kind == "time" && t.active && t.next_at.is_some_and(|n| n <= at))
            .collect();
        for t in &due {
            let (_, every) = time_spec(t);
            match every {
                Some(step) => {
                    // skip past any missed periods, land on the next time strictly after `at`
                    let mut next = t.next_at.unwrap_or(at);
                    while next <= at {
                        next += step;
                    }
                    self.conn.execute(
                        "UPDATE triggers SET fired_at = ?2, next_at = ?3 WHERE id = ?1",
                        params![t.id, at, next],
                    )?;
                }
                None => {
                    self.conn.execute(
                        "UPDATE triggers SET fired_at = ?2, active = 0 WHERE id = ?1",
                        params![t.id, at],
                    )?;
                }
            }
        }
        Ok(due)
    }

    /// The geo triggers that fire on a move from `prev` (None: position unknown until now) to
    /// `pos`: an "enter" trigger fires when the circle is entered, an "exit" when it is left.
    pub fn moved(&self, prev: Option<(f64, f64)>, pos: (f64, f64)) -> Result<Vec<Trigger>> {
        let mut fired = Vec::new();
        for t in self
            .list_triggers()?
            .into_iter()
            .filter(|t| t.kind == "geo" && t.active)
        {
            let Some((lat, lon, radius, on_enter)) = geo_spec(&t) else {
                continue;
            };
            let now_inside = haversine_m(pos.0, pos.1, lat, lon) <= radius;
            let was_inside = prev.map(|p| haversine_m(p.0, p.1, lat, lon) <= radius);
            let crossed = if on_enter {
                now_inside && was_inside != Some(true)
            } else {
                // leaving needs a known previous position inside
                was_inside == Some(true) && !now_inside
            };
            if crossed {
                self.conn.execute(
                    "UPDATE triggers SET fired_at = ?2 WHERE id = ?1",
                    params![t.id, now()],
                )?;
                fired.push(t);
            }
        }
        Ok(fired)
    }

    /// The event triggers for a named event of the system (boot, network…): each is marked fired.
    pub fn fire_event(&self, event: &str) -> Result<Vec<Trigger>> {
        let fired: Vec<Trigger> = self
            .list_triggers()?
            .into_iter()
            .filter(|t| {
                t.kind == "event"
                    && t.active
                    && t.spec.get("event").and_then(Value::as_str) == Some(event)
            })
            .collect();
        for t in &fired {
            self.conn.execute(
                "UPDATE triggers SET fired_at = ?2 WHERE id = ?1",
                params![t.id, now()],
            )?;
        }
        Ok(fired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewNote;
    use serde_json::json;

    fn store_with_note() -> (Store, i64) {
        let s = Store::open_in_memory().unwrap();
        let id = s
            .capture_note(&NewNote {
                text: "note".into(),
                source: "voice".into(),
                ..Default::default()
            })
            .unwrap();
        (s, id)
    }

    #[test]
    fn a_one_shot_time_fires_once_then_switches_off() {
        let (s, note) = store_with_note();
        let t = s
            .create_trigger("time", Some(note), &json!({"at": 1000.0}), Some(1000.0))
            .unwrap();
        assert!(s.due_time(999.0).unwrap().is_empty(), "not yet");
        let fired = s.due_time(1000.0).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, t);
        assert!(
            s.due_time(5000.0).unwrap().is_empty(),
            "does not fire again"
        );
        let after = s.list_triggers().unwrap()[0].clone();
        assert!(!after.active && after.fired_at == Some(1000.0));
    }

    #[test]
    fn a_recurring_time_advances_past_missed_periods() {
        let (s, note) = store_with_note();
        s.create_trigger(
            "time",
            Some(note),
            &json!({"at": 100.0, "every": 60.0}),
            Some(100.0),
        )
        .unwrap();
        // woke up late at 250: it fires once and lands on the next period after 250 (280)
        let fired = s.due_time(250.0).unwrap();
        assert_eq!(fired.len(), 1);
        let t = s.list_triggers().unwrap()[0].clone();
        assert!(t.active);
        assert_eq!(t.next_at, Some(280.0));
        assert!(s.due_time(279.0).unwrap().is_empty());
        assert_eq!(s.due_time(280.0).unwrap().len(), 1);
        assert_eq!(s.list_triggers().unwrap()[0].next_at, Some(340.0));
    }

    #[test]
    fn a_geofence_fires_on_entering_and_on_leaving() {
        let (s, note) = store_with_note();
        // enter home (within 150 m of the point)
        let home = json!({"lat": 48.8566, "lon": 2.3522, "radius_m": 150.0, "on": "enter"});
        s.create_trigger("geo", Some(note), &home, None).unwrap();
        // leave work
        let work = json!({"lat": 45.7640, "lon": 4.8357, "radius_m": 150.0, "on": "exit"});
        let leave = s.create_trigger("geo", Some(note), &work, None).unwrap();

        // first fix, already at home: enter fires (previous position unknown)
        let near_home = (48.85665, 2.35225);
        let f = s.moved(None, near_home).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].spec["on"], "enter");
        // staying inside does not fire again
        assert!(s
            .moved(Some(near_home), (48.85661, 2.35221))
            .unwrap()
            .is_empty());
        // moving from inside work to far away fires the exit trigger
        let at_work = (45.76405, 4.83575);
        let away = (45.80, 4.90);
        let f = s.moved(Some(at_work), away).unwrap();
        assert_eq!(f.iter().map(|t| t.id).collect::<Vec<_>>(), vec![leave]);
        // leaving with no known previous position does not fire exit
        assert!(s.moved(None, away).unwrap().is_empty());
    }

    #[test]
    fn an_event_fires_its_matching_triggers() {
        let (s, note) = store_with_note();
        s.create_trigger("event", Some(note), &json!({"event": "boot"}), None)
            .unwrap();
        s.create_trigger("event", Some(note), &json!({"event": "network"}), None)
            .unwrap();
        let f = s.fire_event("boot").unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].spec["event"], "boot");
        assert!(s.fire_event("sunrise").unwrap().is_empty());
    }
}
