//! Finding a note again: by its words (BM25 over FTS5: no inference, works even when every model
//! is off), by its meaning (KNN by cosine over sqlite-vec), or both fused by Reciprocal Rank Fusion
//! — each list votes `1 / (rrf_k + rank)`, so a note that ranks well either way rises and no score
//! scale has to be made comparable. The query's vector comes from the embedding model the person
//! sets; without one, `hybrid` is words only and `semantic` says it cannot.

use crate::store::Note;
use crate::{clock, Error, Result, Store};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How a search looks for notes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Words and meaning, fused (words only when there is no query vector).
    #[default]
    Hybrid,
    /// Words only (BM25).
    Fts,
    /// Meaning only (cosine).
    Semantic,
}

impl std::str::FromStr for Mode {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim() {
            "" | "hybrid" => Ok(Mode::Hybrid),
            "fts" => Ok(Mode::Fts),
            "semantic" => Ok(Mode::Semantic),
            other => Err(Error::Invalid(format!(
                "a search mode is hybrid, fts or semantic, not {other:?}"
            ))),
        }
    }
}

/// What narrows a search: a time window, a place, tags the note must all carry.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    /// Notes created at or after this time (seconds since the epoch).
    pub since: Option<i64>,
    /// Notes created before this time.
    pub until: Option<i64>,
    /// The note carries every one of these tags.
    pub tags: Vec<String>,
    /// Only these categories (empty: any).
    pub categories: Vec<String>,
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
#[serde(default)]
pub struct SearchOptions {
    pub mode: Mode,
    /// How many hits to return.
    pub limit: usize,
    /// How many to take from each ranking before fusing (a wider pool fuses better).
    pub pool: usize,
    /// RRF's constant (60 is the usual value): larger flattens the weight of the top ranks.
    pub rrf_k: f64,
    pub filters: Filters,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            mode: Mode::Hybrid,
            limit: 5,
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
        let at = clock::parse(&n.created_at).unwrap_or(0);
        if self.since.is_some_and(|s| at < s) || self.until.is_some_and(|u| at >= u) {
            return false;
        }
        if !self.tags.iter().all(|t| n.tags.iter().any(|x| x == t)) {
            return false;
        }
        if !self.categories.is_empty() && !self.categories.contains(&n.category) {
            return false;
        }
        if let Some(g) = &self.near {
            match (n.latitude, n.longitude) {
                (Some(la), Some(lo)) if haversine_m(la, lo, g.lat, g.lon) <= g.radius_m => {}
                _ => return false,
            }
        }
        true
    }

    fn is_empty(&self) -> bool {
        self.since.is_none()
            && self.until.is_none()
            && self.tags.is_empty()
            && self.categories.is_empty()
            && self.near.is_none()
    }
}

/// Fuse ranked lists of ids into one, by RRF; the id and its fused score, best first.
fn rrf(lists: &[&[i64]], rrf_k: f64) -> Vec<(i64, f64)> {
    let mut score: HashMap<i64, f64> = HashMap::new();
    for list in lists {
        for (i, id) in list.iter().enumerate() {
            *score.entry(*id).or_insert(0.0) += 1.0 / (rrf_k + (i + 1) as f64);
        }
    }
    let mut out: Vec<(i64, f64)> = score.into_iter().collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

impl Store {
    /// Search by words, by meaning, or both. `query_vector` is the query embedded by the same model
    /// as the notes (None: no meaning side).
    pub fn search(
        &self,
        query: &str,
        query_vector: Option<&[f32]>,
        opts: &SearchOptions,
    ) -> Result<Vec<Hit>> {
        let f = &opts.filters;
        let limit = opts.limit.max(1);
        // with filters, look further down each list before keeping the first ones that pass
        let pool = if f.is_empty() {
            opts.pool.max(limit)
        } else {
            opts.pool.max(limit) * 4
        };
        let want_words = matches!(opts.mode, Mode::Hybrid | Mode::Fts);
        let want_meaning = matches!(opts.mode, Mode::Hybrid | Mode::Semantic);
        if opts.mode == Mode::Semantic && query_vector.is_none() {
            return Err(Error::Invalid(
                "a search by meaning needs the embedding model: Settings → Kitsune → Embeddings"
                    .into(),
            ));
        }

        let mut notes: HashMap<i64, Note> = HashMap::new();
        let mut keep = |ids: Vec<i64>| -> Result<Vec<i64>> {
            let mut out = Vec::new();
            for n in self.notes_by_ids(&ids)? {
                if f.keep(&n) {
                    out.push(n.id);
                    notes.insert(n.id, n);
                }
            }
            Ok(out)
        };

        let text_ids = if want_words && !query.trim().is_empty() {
            keep(
                self.fts(query, pool)?
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect(),
            )?
        } else {
            Vec::new()
        };
        let vector_ids = match (want_meaning, query_vector) {
            (true, Some(q)) => keep(self.knn(q, pool)?.into_iter().map(|(id, _)| id).collect())?,
            _ => Vec::new(),
        };

        let place = |ids: &[i64]| -> HashMap<i64, usize> {
            ids.iter().enumerate().map(|(i, id)| (*id, i + 1)).collect()
        };
        let (text_place, vector_place) = (place(&text_ids), place(&vector_ids));
        Ok(rrf(&[&text_ids, &vector_ids], opts.rrf_k)
            .into_iter()
            .take(limit)
            .filter_map(|(id, score)| {
                Some(Hit {
                    note: notes.remove(&id)?,
                    score,
                    text_rank: text_place.get(&id).copied(),
                    vector_rank: vector_place.get(&id).copied(),
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewNote, DIM};

    fn note(text: &str, tags: &[&str]) -> NewNote {
        NewNote {
            raw_transcript: text.into(),
            source: "voice".into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// A unit vector mostly along `axis`, a little along `other`.
    fn vec_near(axis: usize, other: usize, w: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; DIM];
        v[axis] = 1.0;
        v[other] = w;
        v
    }

    #[test]
    fn cosine_and_distance_are_sane() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-9);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-9);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0, "sizes differ");
        // Paris to Lyon is about 392 km
        let d = haversine_m(48.8566, 2.3522, 45.7640, 4.8357);
        assert!((390_000.0..395_000.0).contains(&d), "{d}");
        assert_eq!("".parse::<Mode>().unwrap(), Mode::Hybrid);
        assert!("fuzzy".parse::<Mode>().is_err());
    }

    #[test]
    fn words_meaning_and_both_rank_as_asked() {
        let s = Store::open_in_memory().unwrap();
        // "révision" is only in a, but b is nearest in meaning to the query vector
        let a = s
            .insert_note(&note("Révision de la voiture au garage", &[]))
            .unwrap();
        let b = s
            .insert_note(&note("Changer les pneus avant l'hiver", &[]))
            .unwrap();
        let c = s.insert_note(&note("Acheter du pain", &[])).unwrap();
        s.set_embedding(a, &vec_near(1, 0, 0.1)).unwrap();
        s.set_embedding(b, &vec_near(0, 1, 0.1)).unwrap();
        s.set_embedding(c, &vec_near(2, 0, 0.0)).unwrap();
        let q = vec_near(0, 1, 0.0); // closest to b

        let fts = SearchOptions {
            mode: Mode::Fts,
            ..Default::default()
        };
        let words = s.search("révision", Some(&q), &fts).unwrap();
        assert_eq!(
            words.iter().map(|h| h.note.id).collect::<Vec<_>>(),
            vec![a],
            "words only"
        );

        let sem = SearchOptions {
            mode: Mode::Semantic,
            ..Default::default()
        };
        let meaning = s.search("révision", Some(&q), &sem).unwrap();
        assert_eq!(meaning[0].note.id, b, "meaning only");
        assert!(meaning[0].text_rank.is_none());
        assert!(
            s.search("x", None, &sem).is_err(),
            "no vector, no search by meaning"
        );

        let both = s
            .search("révision", Some(&q), &SearchOptions::default())
            .unwrap();
        let ids: Vec<i64> = both.iter().map(|h| h.note.id).collect();
        assert!(ids.contains(&a) && ids.contains(&b));
        assert_eq!(
            both.iter().find(|h| h.note.id == a).unwrap().text_rank,
            Some(1)
        );
        assert_eq!(
            both.iter().find(|h| h.note.id == b).unwrap().vector_rank,
            Some(1)
        );

        // hybrid without a vector is words only: it still answers
        let no_vec = s.search("pain", None, &SearchOptions::default()).unwrap();
        assert_eq!(no_vec[0].note.id, c);
    }

    #[test]
    fn filters_narrow_both_rankings() {
        let s = Store::open_in_memory().unwrap();
        let near = s
            .insert_note(&NewNote {
                latitude: Some(48.8566),
                longitude: Some(2.3522),
                category: "tache".into(),
                ..note("réunion projet", &["travail"])
            })
            .unwrap();
        let far = s
            .insert_note(&NewNote {
                latitude: Some(45.7640),
                longitude: Some(4.8357),
                ..note("réunion projet", &["travail"])
            })
            .unwrap();
        let untagged = s.insert_note(&note("réunion projet", &[])).unwrap();
        for id in [near, far, untagged] {
            s.set_embedding(id, &vec_near(5, 0, 0.0)).unwrap();
        }
        let q = vec_near(5, 0, 0.0);
        let with = |filters: Filters| SearchOptions {
            filters,
            limit: 10,
            ..Default::default()
        };

        let geo = with(Filters {
            near: Some(Geo {
                lat: 48.85,
                lon: 2.35,
                radius_m: 50_000.0,
            }),
            ..Default::default()
        });
        let hits = s.search("réunion", Some(&q), &geo).unwrap();
        assert_eq!(
            hits.iter().map(|h| h.note.id).collect::<Vec<_>>(),
            vec![near]
        );

        let tagged = with(Filters {
            tags: vec!["travail".into()],
            ..Default::default()
        });
        let ids: Vec<i64> = s
            .search("réunion", Some(&q), &tagged)
            .unwrap()
            .iter()
            .map(|h| h.note.id)
            .collect();
        assert!(ids.contains(&near) && ids.contains(&far) && !ids.contains(&untagged));

        let tasks = with(Filters {
            categories: vec!["tache".into()],
            ..Default::default()
        });
        let ids: Vec<i64> = s
            .search("réunion", Some(&q), &tasks)
            .unwrap()
            .iter()
            .map(|h| h.note.id)
            .collect();
        assert_eq!(ids, vec![near]);

        let future = with(Filters {
            since: Some(clock::now() + 3600),
            ..Default::default()
        });
        assert!(s.search("réunion", Some(&q), &future).unwrap().is_empty());
    }

    // the brief's performance targets, on 10 000 notes: words ≤ 5 ms, meaning ≤ 20 ms. Run with
    // `cargo test -p kitsune --release -- --ignored --nocapture` (a debug build is far slower).
    #[test]
    #[ignore]
    fn ten_thousand_notes_are_searched_within_the_budget() {
        let s = Store::open_in_memory().unwrap();
        let words = [
            "café",
            "comptable",
            "loyer",
            "julie",
            "garage",
            "pain",
            "réunion",
            "médecin",
            "clés",
            "train",
        ];
        s.conn.execute_batch("BEGIN").unwrap();
        for i in 0..10_000usize {
            let text = format!("{} {} note {i}", words[i % 10], words[(i / 10) % 10]);
            let id = s.insert_note(&note(&text, &[])).unwrap();
            let mut v = vec![0.0f32; DIM];
            for (j, x) in v.iter_mut().enumerate() {
                *x = (((i * 31 + j * 17) % 97) as f32 / 97.0) - 0.5;
            }
            s.set_embedding(id, &v).unwrap();
        }
        s.conn.execute_batch("COMMIT").unwrap();
        let time = |f: &dyn Fn()| {
            let runs = 20;
            let t = std::time::Instant::now();
            for _ in 0..runs {
                f();
            }
            t.elapsed().as_secs_f64() * 1000.0 / runs as f64
        };
        let fts_ms = time(&|| {
            s.fts("comptable", 5).unwrap();
        });
        let q = vec![0.1f32; DIM];
        let vec_ms = time(&|| {
            s.knn(&q, 5).unwrap();
        });
        println!("10k notes: fts {fts_ms:.2} ms, knn {vec_ms:.2} ms");
        assert!(fts_ms <= 5.0, "fts {fts_ms} ms");
        assert!(vec_ms <= 20.0, "knn {vec_ms} ms");
    }
}
