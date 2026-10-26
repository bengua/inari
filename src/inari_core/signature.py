"""Detached Ed25519 signatures over an exported capsule.

The signature covers the bytes of `manifest.json`, which in turn digests every
other file and the journal: change anything and the signature stops matching.
It lives in `signature/`, which the manifest does not digest (it could not,
without chasing its own tail). A signature attests an exported state; a capsule
that keeps running (holder changes, new events) must be signed again."""

from __future__ import annotations

import base64
import json
from pathlib import Path

from .canon import canonical, sha256_hex

SIG = "signature/manifest.sig.json"


def _crypto():
    try:
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric import ed25519
    except ImportError as exc:  # pragma: no cover - depends on the extra
        raise RuntimeError("install inari-core[signing] to sign or check signatures") from exc
    return serialization, ed25519


def generate_keypair() -> tuple[str, str]:
    """(private, public) as base64 of the raw 32-byte keys."""
    serialization, ed25519 = _crypto()
    private = ed25519.Ed25519PrivateKey.generate()
    raw = serialization.Encoding.Raw, serialization.PrivateFormat.Raw
    priv = private.private_bytes(*raw, serialization.NoEncryption())
    pub = private.public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw
    )
    return base64.b64encode(priv).decode(), base64.b64encode(pub).decode()


def key_id(public_b64: str) -> str:
    return sha256_hex(base64.b64decode(public_b64))[:16]


def _payload(root: Path) -> bytes:
    return canonical({"manifest_sha256": sha256_hex((root / "manifest.json").read_bytes())})


def sign_capsule(root: Path, private_b64: str) -> str:
    """Write the signature; returns the signer's key id."""
    _, ed25519 = _crypto()
    private = ed25519.Ed25519PrivateKey.from_private_bytes(base64.b64decode(private_b64))
    from cryptography.hazmat.primitives import serialization

    pub = base64.b64encode(
        private.public_key().public_bytes(
            serialization.Encoding.Raw, serialization.PublicFormat.Raw
        )
    ).decode()
    signature = base64.b64encode(private.sign(_payload(Path(root)))).decode()
    out = Path(root) / SIG
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(
            {
                "format": "ssc-sig/1",
                "key_id": key_id(pub),
                "public_key": pub,
                "signature": signature,
            },
            indent=2,
        )  # fmt: skip
    )
    return key_id(pub)


def verify_signature(root: Path, trusted: set[str]) -> list[str]:
    """Problems, empty when the capsule is signed by one of the `trusted` public
    keys (base64) and unchanged since."""
    _, ed25519 = _crypto()
    from cryptography.exceptions import InvalidSignature

    path = Path(root) / SIG
    if not path.exists():
        return ["not signed"]
    try:
        doc = json.loads(path.read_text())
        pub = doc["public_key"]
        if pub not in trusted:
            return [f"signed by an untrusted key ({doc.get('key_id', '?')})"]
        key = ed25519.Ed25519PublicKey.from_public_bytes(base64.b64decode(pub))
        key.verify(base64.b64decode(doc["signature"]), _payload(Path(root)))
    except InvalidSignature:
        return ["signature does not match: the capsule changed after signing"]
    except (KeyError, ValueError, OSError) as exc:
        return [f"unreadable signature: {exc}"]
    return []
