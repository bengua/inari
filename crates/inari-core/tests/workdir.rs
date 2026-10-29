//! A thread's folder kept between turns: what changed on disk meanwhile stays, what changed in
//! the thread arrives, and the folder is put away before the thread leaves (or dropped after).

use inari_core::agent::Runner;
use inari_core::capsule::io;
use inari_core::host::CapsuleHost;
use inari_core::openfox::OpenFoxHarness;
use inari_core::teleport::Endpoint;
use std::sync::Arc;

fn tree(host: &CapsuleHost, agent: &str) -> Vec<String> {
    io::load(&host.agents.join(agent))
        .unwrap()
        .snapshot
        .tree
        .keys()
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_kept_folder_keeps_what_changed_on_disk_and_takes_what_changed_in_the_thread() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(CapsuleHost::new("n1", &dir.path().join("node")).unwrap());
    let runner = Runner::new(
        Arc::clone(&host),
        vec![(
            "openfox".to_string(),
            OpenFoxHarness::new(dir.path().join("openfox")),
        )],
        None,
        None,
    );
    let agent = runner.create("t1", "", "a title").unwrap();
    let work = host.open_workdir(&agent, true).unwrap();
    std::fs::write(work.join("a.txt"), "one").unwrap();
    std::fs::write(work.join("gone.txt"), "x").unwrap();
    host.close_workdir(&agent, true).unwrap();
    assert!(work.is_dir(), "kept between turns");
    assert_eq!(tree(&host, &agent), ["a.txt", "gone.txt"]);

    // between turns: a branch changes a.txt on disk; the thread gets b.txt (an attachment, say)
    // and loses gone.txt
    std::fs::write(work.join("a.txt"), "changed on disk").unwrap();
    let other = dir.path().join("elsewhere");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("a.txt"), "one").unwrap();
    std::fs::write(other.join("b.txt"), "new in the thread").unwrap();
    io::update_snapshot(&host.agents.join(&agent), &other).unwrap();

    let work = host.open_workdir(&agent, true).unwrap();
    assert_eq!(
        std::fs::read_to_string(work.join("a.txt")).unwrap(),
        "changed on disk"
    );
    assert_eq!(
        std::fs::read_to_string(work.join("b.txt")).unwrap(),
        "new in the thread"
    );
    assert!(!work.join("gone.txt").exists());
    host.close_workdir(&agent, true).unwrap();

    // before it leaves, what changed since the last turn goes with it
    std::fs::write(work.join("late.txt"), "after the turn").unwrap();
    host.freeze(&agent).await.unwrap();
    assert!(tree(&host, &agent).contains(&"late.txt".to_string()));
    // gone for good: no folder left behind
    host.tombstone(&agent, 2).await.unwrap();
    assert!(!work.exists());
}

#[tokio::test]
async fn a_folder_not_kept_starts_afresh_and_is_put_away() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(CapsuleHost::new("n1", &dir.path().join("node")).unwrap());
    let runner = Runner::new(
        Arc::clone(&host),
        vec![(
            "openfox".to_string(),
            OpenFoxHarness::new(dir.path().join("openfox")),
        )],
        None,
        None,
    );
    let agent = runner.create("t1", "", "a title").unwrap();
    let work = host.open_workdir(&agent, false).unwrap();
    std::fs::write(work.join("a.txt"), "one").unwrap();
    host.close_workdir(&agent, false).unwrap();
    assert!(!work.exists());
    // a stray folder from before is not trusted: the thread's files win
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("stray.txt"), "?").unwrap();
    let work = host.open_workdir(&agent, true).unwrap();
    assert!(!work.join("stray.txt").exists());
    assert!(work.join("a.txt").is_file());
}
