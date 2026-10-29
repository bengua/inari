//! Finding a note again, two ways at once: by its words (BM25, over FTS5) and by its meaning
//! (cosine similarity over the note's embedding). The two rankings are fused by Reciprocal Rank
//! Fusion — each list votes `1 / (rrf_k + rank)` for a note — so a note that ranks well either
//! way rises, and no score scale has to be made comparable. Nothing here is assumed about the
//! embedding model: its name and its vectors come from the caller (set in the GUI, never in the
//! code).

use crate::store::Note;
use crate::{Result, Store};
use serde::{Deserialize, Serialize};

/// What narrows a search: a time window, a place, tags the note must all carry.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Filters {
    /// Notes created at or after this time (seconds since the epoch).
    pub since: Option<f64>,
    /// Notes created before this time.
    pub until: Option<f64>,
    /// The note carries every one of these tags.
    pub tags: Vec<String>,
    /// Within `radius_m` metres of (lat, lon); a note without a place is left out.
    pub near: Option<Geo>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Geo {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
}

/// How a search is run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchOptions {
    /// How many hits to return.
    pub k: usize,
    /// How many to take from each ranking before fusing (a wider pool fuses better).
    pub pool: usize,
    /// RRF's constant: larger flattens the weight of a list's top ranks (OpenFox/Elastic use 60).
    pub rrf_k: f64,
    pub filters: Filters,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            k: 10,
            pool: 100,
            rrf_k: 60.0,
            filters: Filters::default(),
        }
    }
}

/// A note a search found, with how it got there.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hit {
    pub note: Note,
    /// The fused RRF score (higher is better).
    pub score: f64,
    /// Its 1-based place in the word ranking, if it was in it.
    pub text_rank: Option<usize>,
    /// Its 1-based place in the meaning ranking, if it was in it.
    pub vector_rank: Option<usize>,
}

/// Cosine similarity of two vectors (0 when either is zero, or their sizes differ).
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        dot += *x as f64 * *y as f64;
        na += *x as f64 * *x as f64;
        nb += *y as f64 * *y as f64;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Great-circle distance between two points, in metres (haversine).
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().atan2((1.0 - a).sqrt())
}

impl Filters {
    /// Whether a note passes every filter set.
    pub fn keep(&self, n: &Note) -> bool {
        if self.since.is_some_and(|s| n.created_at < s)
            || self.until.is_some_and(|u| n.created_at >= u)
        {
            return false;
        }
        if !self.tags.iter().all(|t| n.tags.iter().any(|x| x == t)) {
            return false;
        }
        if let Some(g) = &self.near {
            match (n.lat, n.lon) {
                (Some(la), Some(lo)) if haversine_m(la, lo, g.lat, g.lon) <= g.radius_m => {}
                _ => return false,
            }
        }
        true
    }
}

/// Fuse ranked lists of ids into one, by RRF; the id and its fused score, best first.
fn rrf(lists: &[&[i64]], rrf_k: f64) -> Vec<(i64, f64)> {
    use std::collections::HashMap;
    let mut score: HashMap<i64, f64> = HashMap::new();
    for list in lists {
        for (i, id) in list.iter().enumerate() {
            *score.entry(*id).or_insert(0.0) += 1.0 / (rrf_k + (i + 1) as f64);
        }
    }
    let mut out: Vec<(i64, f64)> = score.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
    out
}

impl Store {
    /// Search by words and by meaning at once. `query_vector` is the query embedded by the same
    /// model as the notes (None: words only); `model` names which stored vectors to compare.
    pub fn search(
        &self,
        query: &str,
        query_vector: Option<&[f32]>,
        model: &str,
        opts: &SearchOptions,
    ) -> Result<Vec<Hit>> {
        let f = &opts.filters;

        // by words: BM25 over FTS5, then the filters
        let mut text: Vec<Note> = Vec::new();
        if !query.trim().is_empty() {
            for (note, _) in self.search_text(query, opts.pool.max(opts.k))? {
                if f.keep(&note) {
                    text.push(note);
                }
            }
        }
        let text_ids: Vec<i64> = text.iter().map(|n| n.id).collect();

        // by meaning: cosine of the query vector with each stored vector, filters applied
        let mut vector_ids: Vec<i64> = Vec::new();
        if let Some(q) = query_vector.filter(|q| !q.is_empty()) {
            let mut scored: Vec<(Note, f64)> = self
                .embeddings_for_model(model)?
                .into_iter()
                .filter(|(n, _)| f.keep(n))
                .map(|(n, v)| {
                    let s = cosine(q, &v);
                    (n, s)
                })
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.id.cmp(&b.0.id)));
            scored.truncate(opts.pool.max(opts.k));
            vector_ids = scored.iter().map(|(n, _)| n.id).collect();
        }

        let text_place: std::collections::HashMap<i64, usize> = text_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i + 1))
            .collect();
        let vector_place: std::collections::HashMap<i64, usize> = vector_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i + 1))
            .collect();

        let by_id: std::collections::HashMap<i64, Note> =
            text.into_iter().map(|n| (n.id, n)).collect();

        let fused = rrf(&[&text_ids, &vector_ids], opts.rrf_k);
        let mut hits = Vec::new();
        for (id, score) in fused.into_iter().take(opts.k) {
            // a note found only by meaning is not in the text map: read it
            let note = match by_id.get(&id) {
                Some(n) => n.clone(),
                None => match self.get_note(id)? {
                    Some(n) => n,
                    None => continue,
                },
            };
            hits.push(Hit {
                note,
                score,
                text_rank: text_place.get(&id).copied(),
                vector_rank: vector_place.get(&id).copied(),
            });
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewNote;

    fn note(text: &str, tags: &[&str]) -> NewNote {
        NewNote {
            text: text.into(),
            source: "voice".into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn cosine_and_distance_are_sane() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-9);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-9);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0, "sizes differ");
        // Paris to Lyon is about 392 km
        let d = haversine_m(48.8566, 2.3522, 45.7640, 4.8357);
        assert!((390_000.0..395_000.0).contains(&d), "{d}");
    }

    #[test]
    fn words_and_meaning_are_fused_by_rrf() {
        let s = Store::open_in_memory().unwrap();
        // three notes; "révision" is only in a, but b is nearest in meaning to the query vector
        let a = s
            .capture_note(&note("Révision de la voiture au garage", &[]))
            .unwrap();
        let b = s
            .capture_note(&note("Changer les pneus avant l'hiver", &[]))
            .unwrap();
        let c = s.capture_note(&note("Acheter du pain", &[])).unwrap();
        let model = "fake-embed-3";
        s.set_embedding(a, model, &[0.1, 0.9, 0.0]).unwrap();
        s.set_embedding(b, model, &[0.9, 0.1, 0.0]).unwrap();
        s.set_embedding(c, model, &[0.0, 0.0, 1.0]).unwrap();
        let q = [1.0, 0.0, 0.0]; // closest to b

        // words only: a wins (the only one with the word)
        let only_words = s
            .search("révision", None, model, &SearchOptions::default())
            .unwrap();
        assert_eq!(only_words[0].note.id, a);
        assert!(only_words[0].vector_rank.is_none());

        // meaning only: b wins
        let only_meaning = s
            .search("", Some(&q), model, &SearchOptions::default())
            .unwrap();
        assert_eq!(only_meaning[0].note.id, b);

        // both: a and b both surface; c (neither word nor near) is last or absent
        let both = s
            .search("révision", Some(&q), model, &SearchOptions::default())
            .unwrap();
        let ids: Vec<i64> = both.iter().map(|h| h.note.id).collect();
        assert!(ids.contains(&a) && ids.contains(&b));
        assert!(both.iter().find(|h| h.note.id == a).unwrap().text_rank == Some(1));
        assert!(both.iter().find(|h| h.note.id == b).unwrap().vector_rank == Some(1));
        assert!(!ids.contains(&c) || ids.iter().position(|x| *x == c) == Some(ids.len() - 1));
    }

    #[test]
    fn filters_narrow_both_rankings() {
        let s = Store::open_in_memory().unwrap();
        let model = "m";
        let near = s
            .capture_note(&NewNote {
                lat: Some(48.8566),
                lon: Some(2.3522),
                place: "Paris".into(),
                ..note("réunion projet", &["travail"])
            })
            .unwrap();
        let far = s
            .capture_note(&NewNote {
                lat: Some(45.7640),
                lon: Some(4.8357),
                place: "Lyon".into(),
                ..note("réunion projet", &["travail"])
            })
            .unwrap();
        let untagged = s.capture_note(&note("réunion projet", &[])).unwrap();
        for id in [near, far, untagged] {
            s.set_embedding(id, model, &[1.0, 0.0]).unwrap();
        }
        let q = [1.0, 0.0];

        // within 50 km of Paris: only the Paris note
        let geo = SearchOptions {
            filters: Filters {
                near: Some(Geo {
                    lat: 48.85,
                    lon: 2.35,
                    radius_m: 50_000.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let hits = s.search("réunion", Some(&q), model, &geo).unwrap();
        assert_eq!(
            hits.iter().map(|h| h.note.id).collect::<Vec<_>>(),
            vec![near]
        );

        // tag "travail": the two with it, not the untagged one
        let tagged = SearchOptions {
            filters: Filters {
                tags: vec!["travail".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let ids: Vec<i64> = s
            .search("réunion", Some(&q), model, &tagged)
            .unwrap()
            .iter()
            .map(|h| h.note.id)
            .collect();
        assert!(ids.contains(&near) && ids.contains(&far) && !ids.contains(&untagged));
    }
}
