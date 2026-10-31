//! The lists the assistant keeps beside the notes: things to do, the shopping (walked aisle by
//! aisle), the pantry (with best-before days), any other the person names. Kept in `brain.db`.

use crate::store::Store;
use crate::{clock, Error, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const KINDS: [&str; 4] = ["todo", "shopping", "pantry", "custom"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct List {
    pub id: i64,
    pub name: String,
    pub kind: String,
    /// How many items are still open.
    pub open: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: i64,
    pub list: String,
    pub text: String,
    pub qty: Option<f64>,
    pub unit: String,
    pub aisle: String,
    pub due_date: Option<String>,
    pub expires_on: Option<String>,
    pub done: bool,
    pub note_id: Option<i64>,
}

/// An item to add.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewItem {
    pub text: String,
    pub qty: Option<f64>,
    pub unit: String,
    pub aisle: String,
    pub due_date: Option<String>,
    pub expires_on: Option<String>,
    pub note_id: Option<i64>,
}

const ITEM_COLUMNS: &str =
    "i.id, l.name, i.text, i.qty, i.unit, i.aisle, i.due_date, i.expires_on, i.done, i.note_id";

fn item_of(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    Ok(Item {
        id: r.get(0)?,
        list: r.get(1)?,
        text: r.get(2)?,
        qty: r.get(3)?,
        unit: r.get(4)?,
        aisle: r.get(5)?,
        due_date: r.get(6)?,
        expires_on: r.get(7)?,
        done: r.get::<_, i64>(8)? == 1,
        note_id: r.get(9)?,
    })
}

fn day(s: &str) -> bool {
    s.len() == 10 && clock::parse(s).is_some()
}

impl Store {
    /// Every list, with how many of its items are still open.
    pub fn lists(&self) -> Result<Vec<List>> {
        let mut st = self.conn.prepare(
            "SELECT l.id, l.name, l.kind,
                    (SELECT count(*) FROM list_items i WHERE i.list_id = l.id AND i.done = 0)
             FROM lists l ORDER BY l.kind, l.name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(List {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                open: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The list of that name, made (of that kind) when it does not exist yet.
    pub fn ensure_list(&self, name: &str, kind: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("a list has a name".into()));
        }
        if !KINDS.contains(&kind) {
            return Err(Error::Invalid(format!(
                "a list is todo, shopping, pantry or custom, not {kind:?}"
            )));
        }
        if let Some(id) = self
            .conn
            .query_row("SELECT id FROM lists WHERE name = ?1", [name], |r| r.get(0))
            .optional()?
        {
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO lists (name, kind) VALUES (?1, ?2)",
            params![name, kind],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn list_id(&self, name: &str) -> Result<i64> {
        self.conn
            .query_row("SELECT id FROM lists WHERE name = ?1", [name.trim()], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| Error::Invalid(format!("no list {name:?}")))
    }

    /// Add items to a list (made of `kind` if it is new). An item already open on the list with
    /// the same words is not added twice: its quantity grows instead. The ids, in order.
    pub fn add_items(&self, list: &str, kind: &str, items: &[NewItem]) -> Result<Vec<i64>> {
        let list_id = self.ensure_list(list, kind)?;
        let mut ids = Vec::new();
        for it in items {
            let text = it.text.trim();
            if text.is_empty() {
                return Err(Error::Invalid("an item has words".into()));
            }
            if let Some(d) = &it.expires_on {
                if !day(d) {
                    return Err(Error::Invalid(format!("not a day (YYYY-MM-DD): {d:?}")));
                }
            }
            let due = match &it.due_date {
                Some(d) => Some(clock::iso(clock::parse(d).ok_or_else(|| {
                    Error::Invalid(format!("not an ISO 8601 UTC time: {d:?}"))
                })?)),
                None => None,
            };
            let same: Option<(i64, Option<f64>)> = self
                .conn
                .query_row(
                    "SELECT id, qty FROM list_items
                     WHERE list_id = ?1 AND done = 0 AND lower(text) = lower(?2) AND unit = ?3",
                    params![list_id, text, it.unit.trim()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((id, qty)) = same {
                if let (Some(a), Some(b)) = (qty, it.qty) {
                    self.conn.execute(
                        "UPDATE list_items SET qty = ?2 WHERE id = ?1",
                        params![id, a + b],
                    )?;
                }
                ids.push(id);
                continue;
            }
            self.conn.execute(
                "INSERT INTO list_items (list_id, text, qty, unit, aisle, due_date, expires_on, note_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    list_id,
                    text,
                    it.qty,
                    it.unit.trim(),
                    it.aisle.trim(),
                    due,
                    it.expires_on,
                    it.note_id
                ],
            )?;
            ids.push(self.conn.last_insert_rowid());
        }
        Ok(ids)
    }

    /// A list's items: the open ones by aisle (then by due date), and the done ones too if asked.
    pub fn items(&self, list: &str, with_done: bool) -> Result<Vec<Item>> {
        let id = self.list_id(list)?;
        let mut st = self.conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM list_items i JOIN lists l ON l.id = i.list_id
             WHERE i.list_id = ?1 AND (?2 OR i.done = 0)
             ORDER BY i.done, i.aisle, i.due_date IS NULL, i.due_date, i.expires_on IS NULL, i.expires_on, i.id"
        ))?;
        let rows = st.query_map(params![id, with_done], item_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Mark items done (or open again). How many changed.
    pub fn check_items(&self, ids: &[i64], done: bool) -> Result<usize> {
        let at = clock::iso(clock::now());
        let mut n = 0;
        for id in ids {
            n += self.conn.execute(
                "UPDATE list_items SET done = ?2, done_at = CASE ?2 WHEN 1 THEN ?3 ELSE NULL END WHERE id = ?1",
                params![id, done, at],
            )?;
        }
        Ok(n)
    }

    /// Place a task again (a new due date, or none): no penalty, no trace of the old one.
    pub fn set_item_due(&self, id: i64, due: Option<&str>) -> Result<bool> {
        let due = match due {
            Some(d) => Some(clock::iso(clock::parse(d).ok_or_else(|| {
                Error::Invalid(format!("not an ISO 8601 UTC time: {d:?}"))
            })?)),
            None => None,
        };
        Ok(self.conn.execute(
            "UPDATE list_items SET due_date = ?2 WHERE id = ?1",
            params![id, due],
        )? == 1)
    }

    /// The open tasks whose due date has passed (to be placed again, never to be blamed for).
    pub fn overdue(&self, now: i64) -> Result<Vec<Item>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM list_items i JOIN lists l ON l.id = i.list_id
             WHERE i.done = 0 AND i.due_date IS NOT NULL AND i.due_date < ?1
             ORDER BY i.due_date, i.id"
        ))?;
        let rows = st.query_map([clock::iso(now)], item_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// How many items were done since `since` (seconds since the epoch).
    pub fn done_since(&self, since: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM list_items WHERE done = 1 AND done_at >= ?1",
            [clock::iso(since)],
            |r| r.get(0),
        )?)
    }

    pub fn remove_items(&self, ids: &[i64]) -> Result<usize> {
        let mut n = 0;
        for id in ids {
            n += self
                .conn
                .execute("DELETE FROM list_items WHERE id = ?1", [id])?;
        }
        Ok(n)
    }

    /// The pantry items whose best-before day is on or before `day` (YYYY-MM-DD), soonest first.
    pub fn expiring(&self, day: &str) -> Result<Vec<Item>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM list_items i JOIN lists l ON l.id = i.list_id
             WHERE l.kind = 'pantry' AND i.done = 0 AND i.expires_on IS NOT NULL AND i.expires_on <= ?1
             ORDER BY i.expires_on, i.id"
        ))?;
        let rows = st.query_map([day], item_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str) -> NewItem {
        NewItem {
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_shopping_list_merges_and_walks_by_aisle() {
        let s = Store::open_in_memory().unwrap();
        let ids = s
            .add_items(
                "Courses",
                "shopping",
                &[
                    NewItem {
                        text: "Lait".into(),
                        qty: Some(1.0),
                        unit: "L".into(),
                        aisle: "2-frais".into(),
                        ..Default::default()
                    },
                    NewItem {
                        text: "café".into(),
                        aisle: "1-épicerie".into(),
                        ..Default::default()
                    },
                ],
            )
            .unwrap();
        // said again: the same line, more of it
        let again = s
            .add_items(
                "Courses",
                "shopping",
                &[NewItem {
                    text: "lait".into(),
                    qty: Some(2.0),
                    unit: "L".into(),
                    ..Default::default()
                }],
            )
            .unwrap();
        assert_eq!(again[0], ids[0]);
        let items = s.items("Courses", false).unwrap();
        assert_eq!(
            items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            ["café", "Lait"]
        );
        assert_eq!(items[1].qty, Some(3.0));
        assert_eq!(s.lists().unwrap()[0].open, 2);
        assert_eq!(s.check_items(&[ids[1]], true).unwrap(), 1);
        assert_eq!(s.items("Courses", false).unwrap().len(), 1);
        assert_eq!(s.items("Courses", true).unwrap().len(), 2);
        assert_eq!(s.remove_items(&[ids[0]]).unwrap(), 1);
        assert!(s.items("Nope", false).is_err());
        assert!(s.ensure_list("x", "fridge").is_err());
        assert!(s.add_items("Courses", "shopping", &[item(" ")]).is_err());
    }

    #[test]
    fn the_pantry_says_what_expires() {
        let s = Store::open_in_memory().unwrap();
        s.add_items(
            "Garde-manger",
            "pantry",
            &[
                NewItem {
                    text: "yaourts".into(),
                    expires_on: Some("2026-11-01".into()),
                    ..Default::default()
                },
                NewItem {
                    text: "riz".into(),
                    expires_on: Some("2027-06-01".into()),
                    ..Default::default()
                },
                item("sel"),
            ],
        )
        .unwrap();
        let soon = s.expiring("2026-11-03").unwrap();
        assert_eq!(soon.len(), 1);
        assert_eq!(soon[0].text, "yaourts");
        assert!(s
            .add_items(
                "Garde-manger",
                "pantry",
                &[NewItem {
                    text: "x".into(),
                    expires_on: Some("demain".into()),
                    ..Default::default()
                }]
            )
            .is_err());
    }

    #[test]
    fn a_task_is_placed_again_and_the_week_counts_what_was_done() {
        let s = Store::open_in_memory().unwrap();
        let ids = s
            .add_items(
                "Pro",
                "todo",
                &[
                    NewItem {
                        text: "Rappeler la banque".into(),
                        due_date: Some("2026-10-30T09:00:00Z".into()),
                        ..Default::default()
                    },
                    item("Devis"),
                ],
            )
            .unwrap();
        let now = clock::parse("2026-10-31T08:00:00Z").unwrap();
        assert_eq!(s.overdue(now).unwrap()[0].id, ids[0]);
        s.set_item_due(ids[0], Some("2026-10-31T10:00:00Z"))
            .unwrap();
        assert!(s.overdue(now).unwrap().is_empty());
        s.check_items(&[ids[1]], true).unwrap();
        assert_eq!(s.done_since(clock::now() - 7 * 86_400).unwrap(), 1);
    }

    #[test]
    fn a_todo_keeps_its_due_date_in_utc() {
        let s = Store::open_in_memory().unwrap();
        s.add_items(
            "Pro",
            "todo",
            &[NewItem {
                text: "Envoyer le devis".into(),
                due_date: Some("2026-11-02 09:00:00".into()),
                ..Default::default()
            }],
        )
        .unwrap();
        assert_eq!(
            s.items("Pro", false).unwrap()[0].due_date.as_deref(),
            Some("2026-11-02T09:00:00Z")
        );
    }
}
