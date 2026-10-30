//! Reading a thought: a small local language model (≤ 3B, served by llama.cpp, LocalAI, vLLM…)
//! turns what was said into a note — a clean summary, a category, tags, and whether it should come
//! back at a time or at a place.
//!
//! The model is held to a JSON schema through the OpenAI-compatible `response_format`
//! (`json_schema`, which llama.cpp compiles to a GBNF grammar): it cannot answer anything else.
//! Its answer is then parsed strictly with serde — unknown fields refused, no markdown fences
//! stripped, no regex run over it. An answer that does not parse is an error, never repaired.
//!
//! The model does not do date arithmetic, which small models get wrong: it says *which* day
//! (an offset from today, or a calendar date) and *which* part of the day or clock time, and
//! [`resolve_due`] turns that into an ISO 8601 UTC instant. The hours of "morning", "evening"…
//! are settings.

use crate::net;
use crate::{clock, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// How Kitsune reaches the small language model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlmConfig {
    /// The server's base address, e.g. http://127.0.0.1:8081/v1 (empty: notes are kept as said).
    #[serde(default)]
    pub url: String,
    /// The model name the server expects (qwen2.5-3b-instruct, llama-3.2-3b…).
    #[serde(default)]
    pub model: String,
    /// How long a reading may take, in seconds (0: thirty seconds).
    #[serde(default)]
    pub timeout: u64,
    /// The categories a note can fall in; the model must pick one of them.
    #[serde(default = "categories")]
    pub categories: Vec<String>,
    /// The instructions given to the model (empty: the built-in French instructions).
    #[serde(default)]
    pub prompt: String,
    /// The clock time of each part of the day, "HH:MM" in UTC.
    #[serde(default)]
    pub day_parts: DayParts,
}

fn categories() -> Vec<String> {
    ["memo", "tache", "rendez-vous", "course", "idee", "contact"]
        .map(String::from)
        .to_vec()
}

impl Default for SlmConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            model: String::new(),
            timeout: 0,
            categories: categories(),
            prompt: String::new(),
            day_parts: DayParts::default(),
        }
    }
}

/// When "the morning", "the evening"… are, in UTC.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DayParts {
    pub morning: String,
    pub noon: String,
    pub afternoon: String,
    pub evening: String,
    pub night: String,
}

impl Default for DayParts {
    fn default() -> Self {
        Self {
            morning: "09:00".into(),
            noon: "12:00".into(),
            afternoon: "15:00".into(),
            evening: "19:00".into(),
            night: "22:00".into(),
        }
    }
}

impl DayParts {
    fn of(&self, part: &str) -> Option<&str> {
        Some(match part {
            "morning" => &self.morning,
            "noon" => &self.noon,
            "afternoon" => &self.afternoon,
            "evening" => &self.evening,
            "night" => &self.night,
            _ => return None,
        })
    }
}

impl SlmConfig {
    pub fn ready(&self) -> bool {
        !self.url.trim().is_empty()
    }

    fn wait(&self) -> Duration {
        Duration::from_secs(if self.timeout == 0 { 30 } else { self.timeout })
    }
}

/// When a reminder should ring, as the model says it (no arithmetic done by the model).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// A calendar day "YYYY-MM-DD" when one was said, else "".
    pub date: String,
    /// Days from today when no calendar day was said (0 today, 1 tomorrow…).
    pub day_offset: i64,
    /// morning | noon | afternoon | evening | night | none.
    pub part: String,
    /// A clock time "HH:MM" when one was said, else "".
    pub time: String,
}

/// The model's reading of a thought, exactly as the schema allows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reading {
    pub clean_summary: String,
    pub category: String,
    pub tags: Vec<String>,
    /// none | time | geofence.
    pub trigger_type: String,
    pub when: When,
    /// The place to go to, as said ("12 rue Oberkampf, Paris"), for a geofence; else "".
    pub place: String,
}

const PARTS: [&str; 6] = ["morning", "noon", "afternoon", "evening", "night", "none"];

/// The JSON schema the model is held to.
pub fn schema(categories: &[String]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["clean_summary", "category", "tags", "trigger_type", "when", "place"],
        "properties": {
            "clean_summary": {"type": "string", "minLength": 1, "maxLength": 280},
            "category": {"type": "string", "enum": categories},
            "tags": {"type": "array", "maxItems": 6, "items": {"type": "string", "minLength": 1, "maxLength": 32}},
            "trigger_type": {"type": "string", "enum": ["none", "time", "geofence"]},
            "when": {
                "type": "object",
                "additionalProperties": false,
                "required": ["date", "day_offset", "part", "time"],
                "properties": {
                    "date": {"type": "string", "pattern": "^([0-9]{4}-[0-9]{2}-[0-9]{2})?$"},
                    "day_offset": {"type": "integer", "minimum": 0, "maximum": 366},
                    "part": {"type": "string", "enum": PARTS},
                    "time": {"type": "string", "pattern": "^(([01][0-9]|2[0-3]):[0-5][0-9])?$"}
                }
            },
            "place": {"type": "string", "maxLength": 200}
        }
    })
}

const PROMPT: &str =
    "Tu ranges les pensées dictées par une personne TDAH. Tu reçois une transcription \
et tu réponds uniquement par l'objet JSON demandé.\n\
- clean_summary : la pensée reformulée en une phrase courte et actionnable, à l'infinitif si c'est \
une chose à faire, sans les hésitations.\n\
- category : la catégorie la plus juste parmi celles permises.\n\
- tags : 0 à 6 mots-clés en minuscules (personnes, sujets).\n\
- trigger_type : \"time\" si la personne veut qu'on le lui rappelle à un moment, \"geofence\" si \
c'est en arrivant quelque part, sinon \"none\". Mentionner un lieu ne suffit pas : il faut vouloir \
y être rappelé.\n\
- when : pour \"time\", le jour (date AAAA-MM-JJ si une date est dite, sinon day_offset : 0 \
aujourd'hui, 1 demain, 2 après-demain…) et le moment (part : morning, noon, afternoon, evening, \
night) ou l'heure dite (time HH:MM). Sinon date \"\", day_offset 0, part \"none\", time \"\".\n\
- place : pour \"geofence\", le lieu tel qu'il faut le chercher sur une carte (adresse ou nom), \
sinon \"\".";

/// Ask the model to read a transcript. `now` (seconds since the epoch) is told to it so that
/// "lundi" or "le 3" can be placed.
pub fn read(cfg: &SlmConfig, transcript: &str, now: i64) -> Result<Reading> {
    if !cfg.ready() {
        return Err(Error::Invalid(
            "no language model set up: Settings → Kitsune → Understanding".into(),
        ));
    }
    let prompt = if cfg.prompt.trim().is_empty() {
        PROMPT
    } else {
        cfg.prompt.trim()
    };
    let today = &clock::iso(now)[..10];
    let body = json!({
        "model": cfg.model,
        "temperature": 0,
        "messages": [
            {"role": "system", "content": format!("{prompt}\nCatégories permises : {}.", cfg.categories.join(", "))},
            {"role": "user", "content": format!("Aujourd'hui : {today} ({}).\nTranscription : {transcript}", weekday(now))},
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": "note", "strict": true, "schema": schema(&cfg.categories)},
        },
    });
    let url = format!("{}/chat/completions", cfg.url.trim_end_matches('/'));
    let r = net::post_json(&url, &body, &[], cfg.wait())?;
    if !r.ok() {
        return Err(Error::Invalid(format!(
            "the language model answered HTTP {}: {}",
            r.status,
            r.text().chars().take(300).collect::<String>()
        )));
    }
    let reply: Value = serde_json::from_slice(&r.body)?;
    let content = reply["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| Error::Invalid("the language model gave no content".into()))?;
    parse(content, &cfg.categories)
}

/// The model's answer, parsed strictly and checked against what the schema promised. Nothing is
/// cleaned up: an answer wrapped in markdown, with an extra field or a wrong value, is refused.
pub fn parse(content: &str, categories: &[String]) -> Result<Reading> {
    let r: Reading = serde_json::from_str(content).map_err(|e| {
        Error::Invalid(format!(
            "the language model's answer is not the note JSON: {e}"
        ))
    })?;
    let bad = |why: String| Err(Error::Invalid(format!("the language model's answer {why}")));
    if r.clean_summary.trim().is_empty() {
        return bad("has no summary".into());
    }
    if !categories.contains(&r.category) {
        return bad(format!("has an unknown category {:?}", r.category));
    }
    if !["none", "time", "geofence"].contains(&r.trigger_type.as_str()) {
        return bad(format!("has an unknown trigger_type {:?}", r.trigger_type));
    }
    if !PARTS.contains(&r.when.part.as_str()) {
        return bad(format!("has an unknown part of the day {:?}", r.when.part));
    }
    if !(0..=366).contains(&r.when.day_offset) {
        return bad(format!("has a day_offset of {}", r.when.day_offset));
    }
    if !r.when.date.is_empty() && clock::parse(&r.when.date).is_none() {
        return bad(format!("has a date that is not one: {:?}", r.when.date));
    }
    if !r.when.time.is_empty() && hhmm(&r.when.time).is_none() {
        return bad(format!("has a time that is not one: {:?}", r.when.time));
    }
    if r.trigger_type == "geofence" && r.place.trim().is_empty() {
        return bad("asks for a place reminder without a place".into());
    }
    Ok(r)
}

/// "HH:MM" as seconds into the day.
fn hhmm(s: &str) -> Option<i64> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m) && s.len() == 5).then_some(h * 3600 + m * 60)
}

fn weekday(now: i64) -> &'static str {
    // 1970-01-01 was a Thursday
    [
        "jeudi", "vendredi", "samedi", "dimanche", "lundi", "mardi", "mercredi",
    ][now.div_euclid(86_400).rem_euclid(7) as usize]
}

/// The ISO 8601 UTC instant a `when` means, seen from `now`. A calendar date wins over an offset,
/// a clock time over a part of the day; with neither, the morning. A time already gone today
/// (no date said) moves to tomorrow, so a reminder never rings in the past.
pub fn resolve_due(when: &When, parts: &DayParts, now: i64) -> Result<String> {
    let at = if when.time.is_empty() {
        parts.of(&when.part).unwrap_or(&parts.morning)
    } else {
        &when.time
    };
    let secs =
        hhmm(at).ok_or_else(|| Error::Invalid(format!("a time of day that is not one: {at:?}")))?;
    let today = now.div_euclid(86_400) * 86_400;
    let mut due = if when.date.is_empty() {
        today + when.day_offset * 86_400 + secs
    } else {
        clock::parse(&when.date)
            .ok_or_else(|| Error::Invalid(format!("not a date: {:?}", when.date)))?
            + secs
    };
    if when.date.is_empty() && due <= now {
        due += 86_400;
    }
    Ok(clock::iso(due))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_793_386_800; // 2026-10-30T19:00:00Z, a Friday

    fn when(date: &str, day_offset: i64, part: &str, time: &str) -> When {
        When {
            date: date.into(),
            day_offset,
            part: part.into(),
            time: time.into(),
        }
    }

    #[test]
    fn tomorrow_morning_is_nine_utc_the_next_day() {
        let p = DayParts::default();
        assert_eq!(clock::iso(NOW), "2026-10-30T19:00:00Z");
        assert_eq!(weekday(NOW), "vendredi");
        assert_eq!(
            resolve_due(&when("", 1, "morning", ""), &p, NOW).unwrap(),
            "2026-10-31T09:00:00Z"
        );
        assert_eq!(
            resolve_due(&when("", 1, "evening", ""), &p, NOW).unwrap(),
            "2026-10-31T19:00:00Z"
        );
        assert_eq!(
            resolve_due(&when("", 2, "none", "14:30"), &p, NOW).unwrap(),
            "2026-11-01T14:30:00Z"
        );
        assert_eq!(
            resolve_due(&when("2026-12-24", 0, "evening", ""), &p, NOW).unwrap(),
            "2026-12-24T19:00:00Z"
        );
        // "ce matin" said in the evening rings tomorrow morning, never in the past
        assert_eq!(
            resolve_due(&when("", 0, "morning", ""), &p, NOW).unwrap(),
            "2026-10-31T09:00:00Z"
        );
        // "ce soir à 21h" stays today
        assert_eq!(
            resolve_due(&when("", 0, "none", "21:00"), &p, NOW).unwrap(),
            "2026-10-30T21:00:00Z"
        );
        // the hours of the day are settings
        let early = DayParts {
            morning: "07:15".into(),
            ..DayParts::default()
        };
        assert_eq!(
            resolve_due(&when("", 1, "morning", ""), &early, NOW).unwrap(),
            "2026-10-31T07:15:00Z"
        );
    }

    fn cats() -> Vec<String> {
        categories()
    }

    #[test]
    fn the_answer_is_parsed_strictly_and_never_repaired() {
        let good = r#"{"clean_summary":"Appeler le comptable","category":"tache","tags":["comptable"],"trigger_type":"time","when":{"date":"","day_offset":1,"part":"morning","time":""},"place":""}"#;
        let r = parse(good, &cats()).unwrap();
        assert_eq!(
            (r.category.as_str(), r.trigger_type.as_str()),
            ("tache", "time")
        );
        // markdown fences are not stripped
        assert!(parse(&format!("```json\n{good}\n```"), &cats()).is_err());
        // a word before the JSON is not cut away
        assert!(parse(&format!("Voici : {good}"), &cats()).is_err());
        // an extra field is refused
        assert!(parse(
            &good.replace(r#""place":"""#, r#""place":"","mood":"ok""#),
            &cats()
        )
        .is_err());
        // a value outside the schema is refused
        assert!(parse(&good.replace("tache", "urgent"), &cats()).is_err());
        assert!(parse(&good.replace(r#""time":"""#, r#""time":"25:00""#), &cats()).is_err());
        assert!(parse(&good.replace(r#""date":"""#, r#""date":"demain""#), &cats()).is_err());
        assert!(
            parse(
                &good.replace(r#""trigger_type":"time""#, r#""trigger_type":"geofence""#),
                &cats()
            )
            .is_err(),
            "a place reminder needs a place"
        );
        // a missing field is refused
        assert!(parse(&good.replace(r#","place":"""#, ""), &cats()).is_err());
    }

    #[test]
    fn twenty_transcripts_read_by_a_model_parse_as_strict_json() {
        // twenty answers as a schema-constrained 3B model gives them (llama.cpp, json_schema)
        let answers = [
            r#"{"clean_summary":"Appeler le comptable","category":"tache","tags":["comptable"],"trigger_type":"time","when":{"date":"","day_offset":1,"part":"morning","time":""},"place":""}"#,
            r#"{"clean_summary":"Donner le code du portail à Julie","category":"contact","tags":["julie","code"],"trigger_type":"geofence","when":{"date":"","day_offset":0,"part":"none","time":""},"place":"12 rue Oberkampf, Paris"}"#,
            r#"{"clean_summary":"Racheter du café","category":"course","tags":["café"],"trigger_type":"none","when":{"date":"","day_offset":0,"part":"none","time":""},"place":""}"#,
            r#"{"clean_summary":"Le restaurant de la rue de la Paix était excellent","category":"memo","tags":["restaurant"],"trigger_type":"none","when":{"date":"","day_offset":0,"part":"none","time":""},"place":""}"#,
            r#"{"clean_summary":"Prendre les médicaments","category":"tache","tags":["santé"],"trigger_type":"time","when":{"date":"","day_offset":0,"part":"none","time":"21:00"},"place":""}"#,
            r#"{"clean_summary":"Rendez-vous chez le dentiste","category":"rendez-vous","tags":["dentiste","santé"],"trigger_type":"time","when":{"date":"2026-11-04","day_offset":0,"part":"none","time":"10:30"},"place":""}"#,
            r#"{"clean_summary":"Acheter des piles en passant au supermarché","category":"course","tags":["piles"],"trigger_type":"geofence","when":{"date":"","day_offset":0,"part":"none","time":""},"place":"Carrefour Market, Lyon"}"#,
            r#"{"clean_summary":"Idée : une appli pour arroser les plantes","category":"idee","tags":["appli","plantes"],"trigger_type":"none","when":{"date":"","day_offset":0,"part":"none","time":""},"place":""}"#,
            r#"{"clean_summary":"Envoyer le dossier à Marc","category":"tache","tags":["marc","dossier"],"trigger_type":"time","when":{"date":"","day_offset":2,"part":"afternoon","time":""},"place":""}"#,
            r#"{"clean_summary":"Récupérer le colis à la poste","category":"tache","tags":["colis"],"trigger_type":"geofence","when":{"date":"","day_offset":0,"part":"none","time":""},"place":"La Poste, place de la République, Paris"}"#,
            r#"{"clean_summary":"Le numéro de la plombière est dans le carnet bleu","category":"contact","tags":["plombière"],"trigger_type":"none","when":{"date":"","day_offset":0,"part":"none","time":""},"place":""}"#,
            r#"{"clean_summary":"Sortir les poubelles","category":"tache","tags":["maison"],"trigger_type":"time","when":{"date":"","day_offset":0,"part":"evening","time":""},"place":""}"#,
            r#"{"clean_summary":"Payer la cantine","category":"tache","tags":["école","facture"],"trigger_type":"time","when":{"date":"2026-11-02","day_offset":0,"part":"morning","time":""},"place":""}"#,
            r#"{"clean_summary":"Demander à la pharmacie pour l'ordonnance","category":"tache","tags":["pharmacie"],"trigger_type":"geofence","when":{"date":"","day_offset":0,"part":"none","time":""},"place":"Pharmacie du Marché, Nantes"}"#,
            r#"{"clean_summary":"Regarder le film conseillé par Léa","category":"memo","tags":["léa","film"],"trigger_type":"none","when":{"date":"","day_offset":0,"part":"none","time":""},"place":""}"#,
            r#"{"clean_summary":"Rappeler maman","category":"contact","tags":["maman"],"trigger_type":"time","when":{"date":"","day_offset":0,"part":"none","time":"18:00"},"place":""}"#,
            r#"{"clean_summary":"Acheter du pain","category":"course","tags":[],"trigger_type":"none","when":{"date":"","day_offset":0,"part":"none","time":""},"place":""}"#,
            r#"{"clean_summary":"Réunion d'équipe","category":"rendez-vous","tags":["travail"],"trigger_type":"time","when":{"date":"","day_offset":3,"part":"none","time":"09:30"},"place":""}"#,
            r#"{"clean_summary":"Penser à arroser les tomates","category":"tache","tags":["jardin"],"trigger_type":"time","when":{"date":"","day_offset":1,"part":"evening","time":""},"place":""}"#,
            r#"{"clean_summary":"Mot de passe du wifi chez Paul : sur le routeur","category":"memo","tags":["paul","wifi"],"trigger_type":"none","when":{"date":"","day_offset":0,"part":"none","time":""},"place":""}"#,
        ];
        assert_eq!(answers.len(), 20);
        for a in answers {
            let r = parse(a, &cats()).unwrap_or_else(|e| panic!("{a}: {e}"));
            if r.trigger_type == "time" {
                resolve_due(&r.when, &DayParts::default(), NOW).unwrap();
            }
        }
    }

    #[test]
    fn the_schema_holds_the_categories_and_every_field() {
        let s = schema(&cats());
        assert_eq!(s["properties"]["category"]["enum"][1], "tache");
        assert_eq!(s["required"].as_array().unwrap().len(), 6);
        assert_eq!(s["additionalProperties"], false);
    }
}
