# Contributing

## The one rule that matters

**A change is verified by running it.** Before opening a pull request, the thing it
changes has been run, and `./verify.sh` passes.

    uv sync --all-extras
    ./verify.sh          # license, Rust (fmt, clippy, tests), Python (ruff, pytest)

## How work reaches `main`

Nobody pushes to `main`. Open a pull request from a branch; CI runs `./verify.sh`
and `main` only accepts green.

## Standards

- **English** in code, comments, docs, commit messages and anything a tool prints.
- **Tests execute the code.** A faulty path (lost reply, corrupted chunk, stale
  epoch) needs a test that makes it happen, not one that reads the source.
- **Crash-only.** Write files atomically (temp file, then rename). No state a
  restart cannot rebuild.
- **Nothing site-specific.** No hostnames, addresses, account names or secrets in
  code, tests, docs or fixtures — configuration and environment variables instead.
- **Schemas are generated.** After changing a model, run
  `uv run inari-core schema schema` and commit the result; a test fails otherwise.
