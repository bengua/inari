# Contributing

## The one rule that matters

**A change is verified by running it, never by deploying it.** `changed: [gamer]`
proves a file was copied, not that it works. A wrapper on this fleet was once
broken for four hours because it had been deployed and never executed. Before a
pull request is opened, the thing it changes has been run.

## How work reaches `main`

Nobody pushes to `main`. Two paths lead there, and both end in a pull request:

- **An agent run** — `fh-dev <owner/repo> "intention"` delegates the change,
  pushes a branch and opens the PR.
- **Your own work** — `fh-push [-m "title"]` from a clean tree that is ahead of
  `origin/main`, which publishes the branch through the mirror and asks the
  forge authority to open the PR. The pilot workstation holds no token and no
  key by design; the mirror is its only way in.

`fh-push` refuses a dirty tree. That is not pedantry: publishing half a change
leaves the other half asleep on a disk, and nobody notices until the next
divergence.

## Writing an intention for an agent run

Two things decide whether a run succeeds:

**Width.** Runs measured on this fleet: one to two deliverables finish in 6–11
minutes; four finish in about 7; eight expire after 28 minutes having produced
nothing. `fh-dev` refuses an intention it counts at five or more. Name a closed
list of files and nothing else.

Note the counter's blind spot, since it will bite you: it adds *filenames with a
known extension* to *the number of French connectives* (`et`, `puis`, `ensuite`,
`aussi`) minus one. A normally written French paragraph naming one file can be
counted at five. That is a flaw in the counter, not in your intention — rewrite
with fewer connectives rather than forcing with `-f`.

**Evidence.** State the defect as it was observed, with the file and the symptom,
not as a request. "records() sorts by filename, so `audit-*` permanently occupies
the last n positions whatever their date" produces a working fix; "improve
sorting" produces a run that reads the whole repository first.

## Before you open a pull request

- The change has been executed, not only written.
- `README.md` still describes what the repository does, including `Limits`.
- `CHANGELOG.md` has an entry under `Unreleased`.
- `LICENSE` carries no `CHOOSE-A-LICENSE` marker.
- Comments and commit messages are in English. See `docs/CONVENTIONS.md`.

## What CI checks, and what it does not

CI verifies the repository's shape: the required files exist, the license
placeholder is gone, the declared license matches `LICENSE`, and the tests run.

It does not verify that a test looks at the right thing. On 5 September 2026 a
malformed HTTP header reached `main` under a green CI, because the test had been
written alongside the code, forbade itself from touching the network, and never
examined the request it built. A test written with its code inherits that code's
blind spots. Green means the checks passed; it does not mean the change is right.
