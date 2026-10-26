# Template-Repository-GitHub-Flow

The repository every new repository on this forge is generated from, by
`fh-repo-provision.py`. It runs nothing itself: it is the shape a repository is
expected to have on the day it is created, so the shape does not have to be
remembered afterwards.

**Read [`docs/CONVENTIONS.md`](docs/CONVENTIONS.md) first.** It states the rules
— names, language, license, copyright holder, packaging — and, for each one,
where it is enforced.

## Install

Nothing to install. A repository is generated from this one:

    echo '{"name":"<kebab-case-name>","description":"<one sentence>","private":true}' \
      | fh-repo-provision.py

That call creates the repository from this template **and** arms branch
protection on `main` in the same transaction: if the protection fails, the
repository is deleted. No repository can be born without its guard.

## Use

In the generated repository, in this order:

1. **`LICENSE`** — replace the placeholder. The Shape check fails while
   `CHOOSE-A-LICENSE` is present, on purpose: an unlicensed repository should
   not reach `main`.
2. **`pyproject.toml`** — set `name`, `description`, `license` and the
   `[project.scripts]` entry points. Delete the file if the repository ships no
   code. The Shape check refuses a `pyproject.toml` still carrying the
   template's own placeholder name.
3. **`README.md`** — replace this text, keeping the four sections below.
4. **`CHANGELOG.md`** — leave `Unreleased` empty until something belongs in it.

Every README on this forge answers four questions, in this order. A reader who
knows none of the context should be able to stop after any one of them and still
have learned something true.

    ## Install        what it takes to have it
    ## Use            the shortest path to it doing its job
    ## How it works   the mechanism, not the file list
    ## Limits         what it does not do, what is untested, what will bite

`Limits` is checked like the others. It is written last and forgotten first, and
it is the section a future reader thanks you for.

## How it works

Two workflows run on every pull request:

| workflow | status context | what it checks |
|---|---|---|
| `ci.yml` | `CI / lint-test` | sandbox guards, `ruff`, `pytest` — **the context branch protection requires** |
| `shape.yml` | `Shape / Repository shape` | the repository's own shape — files, license, README |

`CI / lint-test` is the context `fh-repo-provision.py` arms on every repository
it generates. Until 6 September 2026 the template carried `lint.yaml` and
`test.yaml` instead, whose contexts were `Lint / lint` and `Test / test`: every
generated repository was born with a guard no workflow could satisfy, and its
first pull request waited on a check that would never report.

The Shape check exists because of what this template used to hand down. Until
September 2026 it carried a one-line README, a `LICENSE` naming a single fixed
holder, and a `pyproject.toml` declaring `name = "ace-agent"` with the
description of an unrelated project. Every repository generated from it
inherited those and shipped with them.

Meanwhile, repositories created by the agent loop did not come through here at
all: `_handle_create_repo` in `fh-forge-helper.py` calls `tea repos create
--private` and produces an *empty* repository, so whatever the first push
contained became the repository. Four repositories created between 31 August and
4 September 2026 arrived with no license and no packaging that way.

The result, measured across eleven repositories: four naming styles, three
README languages, eight missing licenses, and one repository sharing another's
root commit and all 75 of its commits.

## Limits

- **This repository is exempt from its own content checks**, by name. `LICENSE`
  here holds the placeholder that generated repositories must replace. Rename
  this repository and its Shape check turns red until the name in `shape.yml` is
  updated with it.
- **The loop still does not come through here.** `create_repo` in
  `fh-forge-helper.py` must be changed to call `fh-repo-provision.py`; until
  then, a repository born from an agent run is still born empty.
- **A new repository has no way to receive its first commit** through the chain:
  `fh-relais` refuses to push to a default branch, `open_pr` has no base to
  target, and the helper's `push` operation requires an existing `base_sha`.
  The seeding is still manual.
- **Shape checks shape, never correctness.** On 5 September 2026 a malformed
  HTTP header reached `main` under a green CI: the test had been written
  alongside the code, forbade itself from touching the network, and never looked
  at the request it built. A test written with its code inherits that code's
  blind spots.
- **The license comparison is textual.** It looks for a phrase from the
  operative terms plus a size floor. It catches the omission that actually
  happened — the notice shipped instead of the terms — not every mismatch.
