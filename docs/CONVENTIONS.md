# Repository conventions

The rules every repository on this forge follows. They exist because eleven
repositories drifted into four naming styles, three README languages and eight
missing licenses — not through carelessness, but because nothing upstream
required otherwise. Where a rule can be enforced by a validator or by CI, it is;
a rule that depends on memory is a rule that will be broken.

## 1. Names

Lowercase kebab-case, English, matching what the repository delivers.

    fh-coder            good
    FH_CODER            rejected: uppercase and underscore
    fh_voice_console    rejected: underscore

The name must agree with the package it builds. A repository named `FH_CODER`
that installs `fh-coder` forces every reader to learn two names for one thing.

Enforced at creation by `REPO_NAME_RE` in `fh-forge-helper.py` — today that
expression accepts uppercase, dots and underscores, which is why `FH_CODER` and
`fh_voice_coder_console` exist. Restricting it is the upstream half of this rule. Four existing
names predate this rule and keep their French words — `postes`, `parc`,
`tutorat`, `banc-essai`. They are unambiguous and they are the words their
author uses out loud; renaming them would buy uniformity at the cost of a
permanent gap between the repository and the vocabulary of the person working
in it. New repositories do not get that exemption.

## 2. Language

English in everything versioned: code, comments, commit messages, README,
documentation. This is not a preference about English; it is what makes the
work legible to a contributor, a reviewer or a funder who does not read French.

French is allowed as an explicit second document, never mixed into the first:

    README.md        English, canonical, links to the translation on line 3
    README.fr.md     French, complete, links back

A README that switches language mid-paragraph serves neither reader. Two files
serve both.

## 3. License

Every repository carries a `LICENSE` file whose text matches what
`pyproject.toml` declares. Three choices, by what the repository is:

| the repository is | license | why |
|---|---|---|
| software that can be run as a hosted service | `AGPL-3.0-or-later` | a third party hosting a modified version must publish it |
| tooling meant to be copied and adapted | `MIT` | nothing should discourage reuse |
| documentation, not code | `CC-BY-SA-4.0` | a software license fits a document badly |

The full license text, not the notice. The AGPL and the GPL both require the
terms to travel with the work; a file containing only "This program is free
software…" satisfies neither.

## 4. Copyright holder

The `Copyright (c)` line names a holder that can actually enforce it:

- **FreedzHost SASU** — the harness and the fleet tooling. Company work-product.
- **Benoit Guarnaccia** — the personal projects: `purrd`, `inframagotchi`,
  `tutorat`.

`Zordur` stays the public identity in `README.md` and in `pyproject.toml`
authors. A pseudonym is fine as a signature and weak as a title.

## 5. Packaging

A repository that ships something a human runs declares it:

```toml
[project.scripts]
tool-name = "package.module:main"
```

Code invoked by file path is not a product. It cannot be versioned, installed,
upgraded or rolled back — and a change to it can only be verified by running it
in place, which is how four hours were once lost to a wrapper that had been
deployed but never executed.

A repository that ships nothing runnable — pure documentation — has no
`pyproject.toml` and says so in its README.

## 6. What a repository always contains

    README.md                    what it is, who runs it, how
    LICENSE                      full text, matching pyproject
    CHANGELOG.md                 Keep a Changelog, newest first
    CONTRIBUTING.md              how to propose a change here
    .gitignore
    .forgejo/workflows/         ci.yml (CI / lint-test, required), shape.yml
    docs/CONVENTIONS.md         this file

The CI matters beyond testing: branch protection on this forge requires a
status check to exist before a pull request can be merged automatically. A
repository with no workflow can never auto-merge — `fh-coder-mode` is the
standing example, and it is barred from the auto-merge list for exactly that
reason.

## 7. CI constraints on this forge

Learned the expensive way, and they do not change:

- the runner label is `ubuntu-latest`, and only that one
- no slim container image — the runner needs `node` present
- actions are referenced by full URL, not by `owner/name`
