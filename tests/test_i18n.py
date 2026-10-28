"""The page speaks English and French: every text app.js passes to t() has its French."""

from __future__ import annotations

import json
import re
from pathlib import Path

WEB = Path(__file__).resolve().parents[1] / "src" / "inari_core" / "web"


def french() -> dict[str, str]:
    js = (WEB / "i18n.js").read_text()
    body = js.split("/* FR-START */", 1)[1].split("/* FR-END */", 1)[0]
    return json.loads(body)


def first_argument(text: str, start: int) -> str:
    """The source of the first argument of the call whose '(' ends just before `start`."""
    depth, quote, i = 0, None, start
    while i < len(text):
        c = text[i]
        if quote:
            if c == "\\":
                i += 2
                continue
            if c == quote:
                quote = None
        elif c in "\"'`":
            quote = c
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            if depth == 0:
                break
            depth -= 1
        elif c == "," and depth == 0:
            break
        i += 1
    return text[start:i]


def test_every_text_given_to_t_has_its_french():
    js = (WEB / "app.js").read_text()
    fr = french()
    keys = set()
    for m in re.finditer(r"\bt\(", js):
        for lit in re.findall(r'"((?:[^"\\]|\\.)*)"', first_argument(js, m.end())):
            keys.add(lit)
    keys.discard(" pass")  # t(role + " pass"): the full keys are listed below
    keys |= {"supervision pass", "collaboration pass"}
    missing = sorted(k for k in keys if k not in fr)
    assert not missing, f"no French for: {missing}"
    assert len(keys) > 200


def test_the_static_page_is_translated_too():
    html = (WEB / "index.html").read_text()
    fr = french()
    texts = set(re.findall(r'(?:title|aria-label|placeholder)="([^"]+)"', html))
    texts |= {x.strip() for x in re.findall(r">([^<>]*[A-Za-z][^<>]*)<", html) if x.strip()}
    missing = sorted(t for t in texts - {"Inari"} if t not in fr)
    assert not missing, f"no French for: {missing}"


def test_french_keeps_every_placeholder():
    for key, value in french().items():
        assert set(re.findall(r"\{\w+\}", key)) == set(re.findall(r"\{\w+\}", value)), key
