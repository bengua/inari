//! A note's reminder, brought back at the right moment: a time reached, or a place entered. The
//! clock is the core's (it asks for what is due); the place is the phone's (its operating system
//! wakes it when a registered circle is crossed — no GPS polling). A reminder rings once, then stays
//! in the active list until it is dismissed: nothing slips away unseen.

use crate::store::{note_of, Note, NOTE_COLUMNS};
use crate::{clock, Error, Result, Store};
use rusqlite::params;

impl Store {
    /// The reminders still waiting (active), soonest first; only one kind when asked
    /// (`time` or `geofence`).
    pub fn active_reminders(&self, kind: Option<&str>) -> Result<Vec<Note>> {
        let kinds: &[&str] = match kind {
            None => &["time", "geofence"],
            Some("time") => &["time"],
            Some("geofence") => &["geofence"],
            Some(other) => {
                return Err(Error::Invalid(format!(
                    "a reminder is time or geofence, not {other:?}"
                )))
            }
        };
        let mut st = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes
             WHERE status = 'active' AND trigger_type IN (?1, ?2)
             ORDER BY trigger_type = 'geofence', due_date, id"
        ))?;
        let rows = st.query_map(params![kinds[0], kinds[kinds.len() - 1]], note_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The time reminders due at or before `now` (seconds since the epoch) that have not rung yet;
    /// each is marked as rung now. Woken late, everything missed rings at once.
    pub fn due(&self, now: i64) -> Result<Vec<Note>> {
        let at = clock::iso(now);
        let mut st = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes
             WHERE status = 'active' AND trigger_type = 'time' AND due_date <= ?1
               AND notified_at IS NULL
             ORDER BY due_date, id"
        ))?;
        let due: Vec<Note> = st
            .query_map([&at], note_of)?
            .collect::<rusqlite::Result<_>>()?;
        for n in &due {
            self.conn.execute(
                "UPDATE notes SET notified_at = ?2 WHERE id = ?1",
                params![n.id, at],
            )?;
        }
        Ok(due
            .into_iter()
            .map(|mut n| {
                n.notified_at = Some(at.clone());
                n
            })
            .collect())
    }

    /// Snooze a time reminder: it rings again at `due` (ISO 8601 UTC).
    pub fn snooze(&self, id: i64, due: &str) -> Result<bool> {
        let at = clock::parse(due)
            .ok_or_else(|| Error::Invalid(format!("not an ISO 8601 UTC time: {due:?}")))?;
        Ok(self.conn.execute(
            "UPDATE notes SET due_date = ?2, notified_at = NULL, status = 'active'
             WHERE id = ?1 AND trigger_type = 'time'",
            params![id, clock::iso(at)],
        )? == 1)
    }

    /// The active place reminders a point is inside, nearest first (what to tell when the phone
    /// says it entered somewhere).
    pub fn entered(&self, lat: f64, lon: f64) -> Result<Vec<Note>> {
        Ok(self
            .notes_covering(lat, lon)?
            .into_iter()
            .filter(|(n, _)| n.trigger_type == "geofence" && n.status == "active")
            .map(|(n, _)| n)
            .collect())
    }

    /// The notes whose place is still to be found (the geocoder was out of reach).
    pub fn pending_geocode(&self) -> Result<Vec<Note>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE status = 'pending_geocode' ORDER BY id"
        ))?;
        let rows = st.query_map([], note_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::{NewNote, Reminder};
    use crate::Store;

    fn note(text: &str) -> NewNote {
        NewNote {
            raw_transcript: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_time_reminder_rings_once_then_waits_to_be_dismissed() {
        let s = Store::open_in_memory().unwrap();
        let id = s
            .insert_note(&NewNote {
                trigger_type: "time".into(),
                due_date: Some("2026-10-31T10:00:00Z".into()),
                ..note("Rappeler le comptable")
            })
            .unwrap();
        let ten = crate::clock::parse("2026-10-31T10:00:00Z").unwrap();
        assert!(s.due(ten - 1).unwrap().is_empty(), "not yet");
        let rung = s.due(ten).unwrap();
        assert_eq!(rung.len(), 1);
        assert_eq!(rung[0].notified_at.as_deref(), Some("2026-10-31T10:00:00Z"));
        assert!(s.due(ten + 3600).unwrap().is_empty(), "rings once");
        assert_eq!(
            s.active_reminders(Some("time")).unwrap().len(),
            1,
            "still waiting to be dismissed"
        );
        // snoozed, it rings again later
        assert!(s.snooze(id, "2026-10-31T11:00:00Z").unwrap());
        assert_eq!(s.due(ten + 3600).unwrap().len(), 1);
        assert!(s.dismiss(id).unwrap());
        assert!(s.active_reminders(None).unwrap().is_empty());
        assert!(s.active_reminders(Some("weekly")).is_err());
    }

    #[test]
    fn a_place_reminder_answers_where_it_is_and_only_while_active() {
        let s = Store::open_in_memory().unwrap();
        let id = s.insert_note(&note("Code Julie : 45B2")).unwrap();
        s.set_reminder(
            id,
            &Reminder::Geofence {
                latitude: 48.8566,
                longitude: 2.3522,
                radius_meters: 100,
            },
        )
        .unwrap();
        assert_eq!(s.entered(48.8566, 2.3522).unwrap()[0].id, id);
        assert_eq!(s.active_reminders(Some("geofence")).unwrap().len(), 1);
        s.dismiss(id).unwrap();
        assert!(
            s.entered(48.8566, 2.3522).unwrap().is_empty(),
            "dismissed: silent"
        );
        // set again, it is active again
        s.set_reminder(
            id,
            &Reminder::Geofence {
                latitude: 48.8566,
                longitude: 2.3522,
                radius_meters: 100,
            },
        )
        .unwrap();
        assert_eq!(s.entered(48.8566, 2.3522).unwrap().len(), 1);
        s.set_reminder(id, &Reminder::None).unwrap();
        assert!(s.active_reminders(None).unwrap().is_empty());
    }

    #[test]
    fn a_place_still_to_be_found_is_listed() {
        let s = Store::open_in_memory().unwrap();
        let id = s
            .insert_note(&NewNote {
                trigger_type: "geofence".into(),
                status: "pending_geocode".into(),
                location_raw: "14 rue de Paris".into(),
                ..note("Code Julie : 45B2")
            })
            .unwrap();
        assert_eq!(s.pending_geocode().unwrap()[0].id, id);
        assert!(
            s.active_reminders(None).unwrap().is_empty(),
            "not active until placed"
        );
        s.set_reminder(
            id,
            &Reminder::Geofence {
                latitude: 48.85,
                longitude: 2.35,
                radius_meters: 100,
            },
        )
        .unwrap();
        assert!(s.pending_geocode().unwrap().is_empty());
        assert_eq!(s.get_note(id).unwrap().unwrap().status, "active");
    }
}
