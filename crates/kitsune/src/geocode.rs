//! Turning a place said aloud into coordinates with a geocoder the person runs (Nominatim or
//! Photon on the machine or the private network, fed with an OpenStreetMap extract). The public
//! instances are never used: the address would leave the house, and [`crate::net`] refuses it
//! anyway.

use crate::net;
use crate::search::haversine_m;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// How Kitsune reaches the geocoder.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeocoderConfig {
    /// The geocoder's base address, e.g. http://127.0.0.1:8088 (empty: places wait, unresolved).
    #[serde(default)]
    pub url: String,
    /// nominatim | photon.
    #[serde(default = "nominatim")]
    pub kind: String,
    /// How many candidates to ask for.
    #[serde(default = "five")]
    pub limit: u32,
    /// Candidates closer than this to each other (metres) are one place, not an ambiguity.
    #[serde(default = "same_place_m")]
    pub same_place_m: f64,
    /// The radius a new place reminder gets, in metres.
    #[serde(default = "radius_m")]
    pub radius_meters: i64,
    /// How long a lookup may take, in seconds (0: ten seconds).
    #[serde(default)]
    pub timeout: u64,
}

fn nominatim() -> String {
    "nominatim".into()
}
fn five() -> u32 {
    5
}
fn same_place_m() -> f64 {
    250.0
}
fn radius_m() -> i64 {
    crate::store::RADIUS_M
}

impl Default for GeocoderConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            kind: nominatim(),
            limit: five(),
            same_place_m: same_place_m(),
            radius_meters: radius_m(),
            timeout: 0,
        }
    }
}

impl GeocoderConfig {
    pub fn ready(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

/// A place the geocoder offers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub lat: f64,
    pub lon: f64,
    pub label: String,
}

/// What became of a place.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// One place (or one nearest the person): ring there.
    Found(Candidate),
    /// No such place: the reminder cannot be placed.
    NotFound,
    /// Several places far apart and nothing to choose by.
    Ambiguous(Vec<Candidate>),
    /// The geocoder could not be reached: try again later.
    Unreachable(String),
}

/// The candidates for a place, as the geocoder lists them.
pub fn lookup(cfg: &GeocoderConfig, place: &str) -> Result<Vec<Candidate>> {
    if !cfg.ready() {
        return Err(Error::Invalid(
            "no geocoder set up: Settings → Kitsune → Places does not answer".into(),
        ));
    }
    let base = cfg.url.trim_end_matches('/');
    let q = net::encode(place.trim());
    let photon = cfg.kind.eq_ignore_ascii_case("photon");
    let url = if photon {
        format!("{base}/api?q={q}&limit={}", cfg.limit)
    } else {
        format!("{base}/search?q={q}&format=jsonv2&limit={}", cfg.limit)
    };
    let wait = Duration::from_secs(if cfg.timeout == 0 { 10 } else { cfg.timeout });
    let r = net::get(&url, &[], wait)?;
    if !r.ok() {
        return Err(Error::Invalid(format!(
            "the geocoder answered HTTP {}",
            r.status
        )));
    }
    let v: Value = serde_json::from_slice(&r.body)?;
    let num = |x: &Value| {
        x.as_f64()
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    };
    let mut out = Vec::new();
    if photon {
        for f in v["features"].as_array().into_iter().flatten() {
            let c = &f["geometry"]["coordinates"];
            if let (Some(lon), Some(lat)) = (num(&c[0]), num(&c[1])) {
                let p = &f["properties"];
                let label = ["name", "street", "city", "country"]
                    .iter()
                    .filter_map(|k| p[*k].as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push(Candidate { lat, lon, label });
            }
        }
    } else {
        for p in v.as_array().into_iter().flatten() {
            if let (Some(lat), Some(lon)) = (num(&p["lat"]), num(&p["lon"])) {
                let label = p["display_name"].as_str().unwrap_or("").to_string();
                out.push(Candidate { lat, lon, label });
            }
        }
    }
    Ok(out)
}

/// Resolve a place: one result is it; several are it when they are one place, or when the
/// person's last known position picks the nearest; otherwise they are ambiguous.
pub fn resolve(cfg: &GeocoderConfig, place: &str, near: Option<(f64, f64)>) -> Outcome {
    match lookup(cfg, place) {
        // not set up, not answering or answering wrong: the place waits and is tried again
        Err(e) => Outcome::Unreachable(e.to_string()),
        Ok(found) => choose(found, near, cfg.same_place_m),
    }
}

/// Pick among candidates (see [`resolve`]).
pub fn choose(mut found: Vec<Candidate>, near: Option<(f64, f64)>, same_place_m: f64) -> Outcome {
    if found.is_empty() {
        return Outcome::NotFound;
    }
    let first = found[0].clone();
    if found
        .iter()
        .all(|c| haversine_m(first.lat, first.lon, c.lat, c.lon) <= same_place_m)
    {
        return Outcome::Found(first);
    }
    match near {
        Some((lat, lon)) => {
            found.sort_by(|a, b| {
                haversine_m(lat, lon, a.lat, a.lon).total_cmp(&haversine_m(lat, lon, b.lat, b.lon))
            });
            Outcome::Found(found.swap_remove(0))
        }
        None => Outcome::Ambiguous(found),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(lat: f64, lon: f64) -> Candidate {
        Candidate {
            lat,
            lon,
            label: format!("{lat},{lon}"),
        }
    }

    #[test]
    fn several_places_are_one_the_nearest_or_ambiguous() {
        assert_eq!(choose(vec![], None, 250.0), Outcome::NotFound);
        assert_eq!(
            choose(vec![c(48.8566, 2.3522)], None, 250.0),
            Outcome::Found(c(48.8566, 2.3522))
        );
        // two entries 100 m apart are the same place
        let close = vec![c(48.8566, 2.3522), c(48.8575, 2.3522)];
        assert_eq!(
            choose(close, None, 250.0),
            Outcome::Found(c(48.8566, 2.3522))
        );
        // a rue de la Paix in Paris and one in Nantes: the person's position picks
        let far = vec![c(48.8687, 2.3317), c(47.2138, -1.5566)];
        assert_eq!(
            choose(far.clone(), Some((47.21, -1.55)), 250.0),
            Outcome::Found(c(47.2138, -1.5566))
        );
        assert!(matches!(choose(far, None, 250.0), Outcome::Ambiguous(v) if v.len() == 2));
    }

    #[test]
    fn without_a_geocoder_a_place_waits() {
        let out = resolve(&GeocoderConfig::default(), "12 rue Oberkampf", None);
        assert!(matches!(out, Outcome::Unreachable(_)));
    }
}
