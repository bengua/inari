//! The per-session bubble: a write outside the workdir is impossible (read-only
//! filesystem, errno), a write inside it lands on the host, and a node without
//! bwrap refuses to wrap at all (the thread never starts unsandboxed).

use inari_core::sandbox;
use std::process::Command;

fn run(argv: &[String]) -> std::process::Output {
    Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .expect("the bubble runs")
}

#[test]
fn a_write_outside_the_workdir_is_impossible_a_write_inside_lands() {
    if !sandbox::available() {
        eprintln!("skipped: bwrap not installed on this machine");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let outside = dir.path().join("outside.txt");
    std::fs::create_dir_all(&work).unwrap();

    // outside the workdir: read-only filesystem, the command fails and nothing lands
    let argv = sandbox::wrap(
        &[
            "sh".into(),
            "-c".into(),
            format!("echo secret > '{}'", outside.display()),
        ],
        &work,
        &[],
    )
    .unwrap();
    let out = run(&argv);
    assert!(
        !out.status.success(),
        "a write outside the workdir must fail: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!outside.exists(), "nothing may land outside the workdir");

    // inside the workdir: the write passes and is visible from the host
    let argv = sandbox::wrap(
        &["sh".into(), "-c".into(), "echo ok > inside.txt".into()],
        &work,
        &[],
    )
    .unwrap();
    let out = run(&argv);
    assert!(
        out.status.success(),
        "a write inside the workdir must pass: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(work.join("inside.txt"))
            .unwrap()
            .trim(),
        "ok"
    );
}

#[test]
fn an_extra_rw_path_stays_writable_the_rest_read_only() {
    if !sandbox::available() {
        eprintln!("skipped: bwrap not installed on this machine");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let home = dir.path().join("pass-home");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&home).unwrap();

    // the pass's own HOME (extra_rw) accepts writes…
    let argv = sandbox::wrap(
        &[
            "sh".into(),
            "-c".into(),
            format!("echo ok > '{}/token'", home.display()),
        ],
        &work,
        std::slice::from_ref(&home),
    )
    .unwrap();
    let out = run(&argv);
    assert!(
        out.status.success(),
        "extra_rw must stay writable: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(home.join("token")).unwrap().trim(),
        "ok"
    );

    // …but its parent directory does not
    let argv = sandbox::wrap(
        &[
            "sh".into(),
            "-c".into(),
            format!("echo no > '{}/nope.txt'", dir.path().display()),
        ],
        &work,
        std::slice::from_ref(&home),
    )
    .unwrap();
    let out = run(&argv);
    assert!(!out.status.success(), "outside extra_rw stays read-only");
    assert!(!dir.path().join("nope.txt").exists());
}

#[test]
fn without_bwrap_the_thread_is_refused_and_told_why() {
    let dir = tempfile::tempdir().unwrap();
    let err = sandbox::wrap_with(&["true".into()], dir.path(), &[], false)
        .expect_err("no bwrap, no session");
    assert!(
        err.to_string().contains("bwrap"),
        "the refusal names the missing piece: {err}"
    );
}

#[test]
fn an_empty_command_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    assert!(sandbox::wrap_with(&[], dir.path(), &[], true).is_err());
}
