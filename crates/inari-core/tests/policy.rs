//! The policy engine (src/policy.rs): three levels, the strictest verdict wins; budgets
//! with a soft threshold and a hard cap; the sudo gate always asks the owner with the
//! exact command; a signed verdict counts once, before it expires.

use inari_core::approvals::{sign, NodeApprovals};
use inari_core::bus::Bus;
use inari_core::policy::{classify, record, Class, Level, Policy, Request, Spend, Verdict};
use serde_json::{json, Map, Value};
use std::sync::Arc;

fn req(command: &str, auto: bool, spend: Spend) -> Request<'_> {
    Request {
        command,
        auto_requested: auto,
        spend,
    }
}

fn free() -> Policy {
    Policy::default() // no level constrains anything: the pre-policy behavior
}

#[test]
fn the_command_is_read_past_its_tool_kind() {
    assert_eq!(classify("bash: ls -la"), Class::Benign);
    assert_eq!(classify("ls"), Class::Benign);
    assert_eq!(classify("bash: sudo rm -rf /"), Class::Sudo);
    assert_eq!(classify("sudo -n true"), Class::Sudo);
    assert_eq!(classify("run: /usr/bin/doas ls"), Class::Sudo);
    assert_eq!(classify("bash: rm -rf x"), Class::Regular);
    // shell trickery demotes an otherwise read-only command
    assert_eq!(classify("bash: ls; rm -rf x"), Class::Regular);
    assert_eq!(classify("bash: cat x > /etc/y"), Class::Regular);
    assert_eq!(classify("bash: echo $(whoami)"), Class::Regular);
    // git is read-only only for its read-only subcommands
    assert_eq!(classify("bash: git status"), Class::Benign);
    assert_eq!(classify("bash: git push --force"), Class::Regular);
}

#[test]
fn without_any_limit_the_old_behavior_is_kept() {
    let p = free();
    // auto approves, everything else asks, nothing is denied
    assert_eq!(
        p.decide(&req("bash: rm -rf x", true, Spend::default())),
        Verdict::Allow
    );
    assert_eq!(
        p.decide(&req("bash: rm -rf x", false, Spend::default())),
        Verdict::Ask("approval required".into())
    );
    // even benign commands ask, unless a level says otherwise
    assert!(matches!(
        p.decide(&req("bash: ls", false, Spend::default())),
        Verdict::Ask(_)
    ));
}

#[test]
fn the_strictest_level_wins() {
    let ask = Level {
        allow_benign: Some(true),
        ..Default::default()
    };
    let no_auto = Level {
        auto: Some(false),
        ..Default::default()
    };
    // one level allows benign, the others say nothing: benign passes
    let p = Policy {
        server: ask.clone(),
        ..Default::default()
    };
    assert_eq!(
        p.decide(&req("bash: ls", false, Spend::default())),
        Verdict::Allow
    );
    // one level forbids auto: auto is off even if another permits it
    let p = Policy {
        server: no_auto,
        agent: Level {
            auto: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        p.decide(&req("bash: rm -rf x", true, Spend::default())),
        Verdict::Ask(_)
    ));
    // a Deny anywhere beats an Allow everywhere else
    let p = Policy {
        server: Level {
            tokens_cap: Some(10),
            ..Default::default()
        },
        session: ask,
        ..Default::default()
    };
    let spent = Spend {
        tokens: 11,
        gpu_seconds: 0.0,
    };
    assert!(matches!(
        p.decide(&req("bash: ls", false, spent)),
        Verdict::Deny(_)
    ));
}

#[test]
fn a_spent_budget_denies_a_soft_one_asks() {
    let spent = Spend {
        tokens: 150_000,
        gpu_seconds: 700.0,
    };
    let p = Policy {
        server: Level {
            tokens_cap: Some(100_000),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        p.decide(&req("bash: rm -rf x", false, spent)),
        Verdict::Deny("token budget spent (150000 > 100000)".into())
    );
    let p = Policy {
        agent: Level {
            tokens_ask: Some(100_000),
            gpu_ask: Some(600.0),
            ..Default::default()
        },
        ..Default::default()
    };
    // over the soft threshold even auto mode asks: a budget beats convenience
    assert!(matches!(
        p.decide(&req("bash: rm -rf x", true, spent)),
        Verdict::Ask(_)
    ));
    // the hard GPU cap denies too
    let p = Policy {
        session: Level {
            gpu_cap: Some(600.0),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        p.decide(&req("bash: rm -rf x", false, spent)),
        Verdict::Deny(_)
    ));
    // under the budgets, life goes on
    let p = Policy {
        server: Level {
            tokens_cap: Some(1_000_000),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        p.decide(&req("bash: rm -rf x", false, spent)),
        Verdict::Ask(_)
    ));
    assert_eq!(
        p.decide(&req("bash: rm -rf x", true, spent)),
        Verdict::Allow
    );
}

#[test]
fn sudo_always_asks_with_the_exact_command_even_in_auto() {
    let p = Policy {
        server: Level {
            auto: Some(true),
            allow_benign: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        p.decide(&req("bash: sudo rm -rf /var/lib/x", true, Spend::default())),
        Verdict::Ask("sudo gate: sudo rm -rf /var/lib/x".into())
    );
    // sudo is never denied by budgets either: the owner decides, on the exact command
    let p = Policy {
        server: Level {
            tokens_cap: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };
    let spent = Spend {
        tokens: 2,
        gpu_seconds: 0.0,
    };
    assert_eq!(
        p.decide(&req("bash: sudo apt update", false, spent)),
        Verdict::Ask("sudo gate: sudo apt update".into())
    );
}

#[test]
fn the_session_level_comes_from_the_labels() {
    let mut labels = Map::new();
    labels.insert("policy.tokens_cap".into(), json!("100"));
    labels.insert("policy.auto".into(), json!("0"));
    let level = Level::from_labels(&labels);
    assert_eq!(level.tokens_cap, Some(100));
    assert_eq!(level.auto, Some(false));
    let p = Policy {
        session: level,
        ..Default::default()
    };
    let spent = Spend {
        tokens: 101,
        gpu_seconds: 0.0,
    };
    assert!(matches!(
        p.decide(&req("bash: ls", false, spent)),
        Verdict::Deny(_)
    ));
}

#[test]
fn the_record_matches_the_schema() {
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../schema/policy-decision.schema.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let r = record(
        "t1",
        "gamer",
        "bash: sudo apt update",
        &Verdict::Ask("sudo gate: sudo apt update".into()),
        &Spend {
            tokens: 42,
            gpu_seconds: 3.5,
        },
        1_762_000_000.0,
    );
    for key in schema["required"].as_array().unwrap() {
        assert!(r.get(key.as_str().unwrap()).is_some(), "missing {key}");
    }
    assert!(
        ["benign", "regular", "sudo"].contains(&r["class"].as_str().unwrap()),
        "class enum"
    );
    assert!(
        ["allow", "ask", "deny"].contains(&r["verdict"].as_str().unwrap()),
        "verdict enum"
    );
    assert!(r["spend"]["tokens"].is_u64() && r["spend"]["gpu_seconds"].is_f64());
}

#[tokio::test]
async fn a_verdict_the_policy_asked_for_counts_once_and_expires() {
    // the Ask side of the engine flows through NodeApprovals: signed, once, in time
    let bus = Bus::new("n");
    let approvals = NodeApprovals::start(Arc::clone(&bus), b"key");
    let mut questions = bus.subscribe("approval.request");
    let a = Arc::clone(&approvals);
    let asked = tokio::spawn(async move { a.ask("t1", "n", "sudo apt update", 0.4).await });
    let q = questions.recv().await.unwrap().payload;
    // the exact command is what the owner signs for
    assert_eq!(q["command"], "sudo apt update");
    let good = json!({"request_id": q["request_id"], "approved": true,
                      "signature": sign(&q, true, b"key")});
    assert!(approvals.resolve(&good));
    assert!(!approvals.resolve(&good), "a verdict counts once");
    assert!(asked.await.unwrap());
    // an expired question is denied by silence
    assert!(!approvals.ask("t1", "n", "sudo apt update", 0.1).await);
}
