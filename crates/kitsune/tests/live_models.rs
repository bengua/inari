//! The spec's checks against the person's real local models, run on the machine that serves
//! them (they are ignored in CI, which has none):
//!
//! ```text
//! KITSUNE_SLM_URL=http://127.0.0.1:8081/v1 KITSUNE_SLM_MODEL=qwen2.5-3b-instruct \
//! KITSUNE_EMBED_URL=http://127.0.0.1:8082/v1 KITSUNE_EMBED_MODEL=multilingual-e5-small \
//!   cargo test -p kitsune --release --test live_models -- --ignored --nocapture
//! ```

use kitsune::understand::{self, DayParts, SlmConfig};
use kitsune::{clock, embed, EmbedConfig, DIM};
use std::time::Instant;

const TRANSCRIPTS: [&str; 20] = [
    "euh rappelle-moi d'appeler le comptable demain matin",
    "quand j'arrive chez Julie au 12 rue Oberkampf faut que je lui donne le code du portail",
    "faut que je pense à racheter du café",
    "le resto de la rue de la Paix était vraiment top",
    "prendre mes médicaments ce soir à 21 heures",
    "rendez-vous chez le dentiste le 4 novembre à 10 heures 30",
    "quand je passe au Carrefour Market acheter des piles",
    "idée : une appli qui rappelle d'arroser les plantes",
    "envoyer le dossier à Marc après-demain dans l'après-midi",
    "récupérer le colis à la poste de la place de la République",
    "le numéro de la plombière est dans le carnet bleu",
    "sortir les poubelles ce soir",
    "payer la cantine lundi matin",
    "en arrivant à la pharmacie demander pour l'ordonnance",
    "regarder le film que Léa m'a conseillé",
    "rappeler maman à 18 heures",
    "acheter du pain",
    "réunion d'équipe dans trois jours à 9 heures 30",
    "penser à arroser les tomates demain soir",
    "le mot de passe du wifi chez Paul est sur le routeur",
];

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

#[test]
#[ignore = "needs the person's local language model (KITSUNE_SLM_URL)"]
fn twenty_transcripts_parse_as_strict_json_on_the_real_model() {
    let Some(url) = env("KITSUNE_SLM_URL") else {
        panic!("set KITSUNE_SLM_URL (and KITSUNE_SLM_MODEL)");
    };
    let cfg = SlmConfig {
        url,
        model: env("KITSUNE_SLM_MODEL").unwrap_or_default(),
        timeout: 60,
        ..Default::default()
    };
    let mut slowest = 0u128;
    for t in TRANSCRIPTS {
        let started = Instant::now();
        let r = understand::read(&cfg, t, clock::now()).unwrap_or_else(|e| panic!("{t:?}: {e}"));
        slowest = slowest.max(started.elapsed().as_millis());
        if r.trigger_type == "time" {
            understand::resolve_due(&r.when, &DayParts::default(), clock::now()).unwrap();
        }
        println!("{t:?}\n  → {}", serde_json::to_string(&r).unwrap());
    }
    println!("slowest reading: {slowest} ms");
}

#[test]
#[ignore = "needs the person's local embedding server (KITSUNE_EMBED_URL)"]
fn the_real_embedding_model_is_384_wide() {
    let Some(url) = env("KITSUNE_EMBED_URL") else {
        panic!("set KITSUNE_EMBED_URL (and KITSUNE_EMBED_MODEL)");
    };
    let cfg = EmbedConfig {
        url,
        model: env("KITSUNE_EMBED_MODEL").unwrap_or_default(),
        timeout: 30,
        ..Default::default()
    };
    let started = Instant::now();
    let v = embed::embed(&cfg, "passage: Racheter du café").unwrap();
    println!("one embedding: {} ms", started.elapsed().as_millis());
    assert_eq!(v.len(), DIM);
}
