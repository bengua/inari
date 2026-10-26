#!/usr/bin/env bash
# What must be true of a repository born from this template — runnable by hand
# and by the CI, identically. The CI only calls this file.
#
# Sections: the sandbox is not visible; shipped code carries no sandbox path;
# the repository has its shape (files, license terms); then uv sync, ruff,
# pytest. The shape checks used to be a second workflow (`Shape`) producing a
# second status context that the protection did not require: a control nobody
# waits on is runner time and a red badge for nothing.
set -u
ROUGE() { echo "$1"; echo "ROUGE"; exit 1; }
DIR=$(cd "$(dirname "$0")" && pwd)
cd "$DIR"

# 1. The CI runner must not see the sandbox: one that does is not isolated.
#    In CI only — the compute node runs this same file from /srv/devlab/runs
#    when the chain verifies a repository, and refusing there made every such
#    run red for a reason that has nothing to do with the code (measured on
#    fh-seq, 9 September 2026). A guard that fires where it does not apply
#    teaches people to bypass it.
if [ "${GITHUB_ACTIONS:-}" = true ] && [ -e /srv/devlab ]; then
  ROUGE "/srv/devlab is visible to CI: this runner is not isolated"
fi
echo "OK  sandbox absent"

# 2. No hardcoded sandbox path in shipped code.
targets=""
for d in src scripts integrations; do [ -d "$d" ] && targets="$targets $d"; done
if [ -n "$targets" ]; then
  if grep -rnI --exclude-dir=__pycache__ --exclude=setup-devlab.sh -e '/srv/devlab' -e '/home/coder' $targets; then
    ROUGE "hardcoded sandbox path in shipped code: use \$HOME, XDG_RUNTIME_DIR or configuration"
  fi
fi
echo "OK  no sandbox paths"

# 3. Shape. The template itself is exempt from the content checks (it ships
#    the placeholders on purpose); a repository born from it is not.
nom=$(basename "${GITHUB_REPOSITORY:-${FORGEJO_REPOSITORY:-$(git remote get-url origin 2>/dev/null | sed -E 's#\.git$##; s#.*/##')}}")
[ -n "$nom" ] || nom=$(basename "$DIR")
exempt=0; [ "$nom" = "Template-Repository-GitHub-Flow" ] && exempt=1
manquant=0
for f in README.md LICENSE CHANGELOG.md CONTRIBUTING.md .gitignore; do
  [ -f "$f" ] || { echo "missing: $f"; manquant=1; }
done
[ "$manquant" = 0 ] || ROUGE "required files missing"
if [ "$exempt" = 0 ]; then
  grep -q 'CHOOSE-A-LICENSE' LICENSE && ROUGE "LICENSE still holds the placeholder — see docs/CONVENTIONS.md section 3"
  if [ -f pyproject.toml ]; then
    declaree=$(grep -m1 -E '^\s*license\s*=' pyproject.toml | grep -oE '[A-Za-z0-9.-]+-or-later|MIT|CC-BY-SA-4\.0|AGPL-3\.0[A-Za-z-]*|GPL-3\.0[A-Za-z-]*' | head -1)
    [ -n "$declaree" ] || ROUGE "pyproject.toml declares no license"
    case "$declaree" in
      MIT)          motif='Permission is hereby granted, free of charge' ;;
      AGPL-3.0*)    motif='END OF TERMS AND CONDITIONS' ;;
      GPL-3.0*)     motif='END OF TERMS AND CONDITIONS' ;;
      CC-BY-SA-4.0) motif='Section 1' ;;
      *)            ROUGE "unknown license: $declaree" ;;
    esac
    grep -qF "$motif" LICENSE || ROUGE "LICENSE is missing '$motif': it holds the notice, not the terms (docs/CONVENTIONS.md section 3)"
    # A floor of ten thousand bytes let an ABRIDGED GPL through: the local
    # coder wrote one of 16 438 bytes with twelve numbered sections where the
    # real text has 34 674 and nineteen, and it shipped — it had the closing
    # line and the size. A licence that ships incomplete does not grant what
    # it says it grants. The floor is now what the licence actually weighs,
    # and the numbered terms are counted: a summary cannot fake nineteen.
    case "$declaree" in
      AGPL-3.0*|GPL-3.0*)
        taille=$(wc -c < LICENSE)
        [ "$taille" -ge 30000 ] || ROUGE "LICENSE is ${taille} bytes: $declaree is about 34 700 — this is an abridged text, not the licence"
        # Measured, not remembered: the GPL-3.0 carries 18 numbered clauses (0
        # to 17) and the AGPL 19, with no fixed indentation — the first version
        # of the check looked for an indented pattern and counted zero on the
        # real text, which would have accused every licence in the fleet.
        clauses=$(grep -cE '^[[:space:]]*[0-9]+\. ' LICENSE || true)
        [ "$clauses" -ge 17 ] || ROUGE "LICENSE carries ${clauses} numbered terms: $declaree has at least eighteen — this is a summary, not the licence" ;;
      CC-BY-SA-4.0)
        [ "$(wc -c < LICENSE)" -ge 10000 ] || ROUGE "LICENSE is too short for $declaree" ;;
    esac
    echo "OK  license $declaree, terms present"
  fi
fi
if [ "$exempt" = 1 ]; then echo "OK  shape (template: content checks skipped)"; else echo "OK  shape"; fi

# 4. Code: whichever this repository actually ships. A repository with no
#    pyproject.toml and no package.json ships no code (a documentation
#    repository says so in its README, per docs/CONVENTIONS.md section 5).
if [ -f pyproject.toml ]; then
  command -v uv >/dev/null || ROUGE "uv absent: install it (the runner image carries it)"
  uv sync --all-extras -q || ROUGE "uv sync failed"
  uv run ruff check . || ROUGE "ruff"
  timeout 300 uv run pytest -q || ROUGE "pytest"
  echo "OK  code (python: ruff, pytest)"
elif [ -f package.json ]; then
  # Same two questions as the Python branch — does it lint, does it pass —
  # answered with whatever the repository itself declares. A script absent
  # from package.json is not a failure: pytest with no tests collected is
  # not one either, and a repository without a `test` script yet should not
  # be told it failed a test it never had.
  has_script() {
    python3 -c "
import json, sys
try:
    scripts = json.load(open('package.json')).get('scripts', {})
except (OSError, ValueError):
    scripts = {}
sys.exit(0 if sys.argv[1] in scripts else 1)
" "$1"
  }
  command -v npm >/dev/null || ROUGE "npm absent: install it (the runner image carries it)"
  npm ci --no-audit --no-fund --silent || ROUGE "npm ci failed"
  if has_script lint; then
    npm run --silent lint || ROUGE "lint"
  elif [ -f tsconfig.json ]; then
    npx --no-install tsc --noEmit || ROUGE "tsc"
  fi
  if has_script test; then
    timeout 300 npm test --silent || ROUGE "test"
  fi
  echo "OK  code (node: $(has_script lint && echo lint || { [ -f tsconfig.json ] && echo tsc || echo "no lint script"; }), $(has_script test && echo test || echo "no test script"))"
else
  echo "OK  code (no pyproject.toml or package.json: documentation repository)"
fi
# English throughout. english-check (in the runner image) counts French prose,
# French-built names and French file names; the count may not grow, and every
# translation lowers the allowance in the same change. Legitimately French
# files are listed in .english-exceptions, each with its reason.
command -v english-check >/dev/null || ROUGE "english-check absent (the runner image carries it)"
english-check --max 0 || ROUGE "english-check: the French count changed (english-check --list)"
echo "OK  english"
echo "VERT"
