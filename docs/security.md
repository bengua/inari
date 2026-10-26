# Security

## Bring your own keys

- A capsule contains **names** of secrets (`environment.secret_refs`), never values.
  `EnvironmentSpec` rejects an `env` entry whose key looks like a secret.
- Values live in the host's keychain (`KeyringResolver`, optional extra
  `inari-core[keyring]`) or, in CI and containers, in `INARI_SECRET_*` variables
  (`EnvResolver`).
- `ephemeral_env(spec, resolver)` builds the environment for **one child process**.
  A missing secret is reported by name only.
- Secrets never cross the mesh: a capsule moves with its names; the target host
  resolves them from its own keychain, or the move fails in `prepare`, before the
  point of no return.
- `leaks(root, values)` scans a directory for secret values and `scrub(text,
  values)` redacts them; the test-suite uses both to prove that a packed capsule
  contains no value.

## Approvals

For commands matching `AccessPolicy.approval_required`, the node opens an
`ApprovalRequest` (id, agent, command, nonce, expiry). The phone answers with an
`ApprovalResponse` whose signature is HMAC-SHA256 over `(request id, nonce,
decision)` with a key shared between the device and the node (provisioned once,
stored in each side's keychain).

- accepted **once** — a replay finds no pending request;
- accepted only **before the expiry**;
- the decision is inside the signature, so a "no" cannot be relabelled "yes";
- compared in constant time;
- **no answer means no**: `decide` returns `False` on timeout.

Biometric confirmation is the phone's job (it gates use of the key); the node only
checks the signature.

## Approvals over the bus

`NodeApprovals.ask(...)` publishes `approval.request` and waits; `PhoneApprover`
answers on `approval.response` with the signed verdict. Both subjects are durable:
a phone that was offline receives the question when it reconnects, and an answer
that arrives after the expiry is refused. A server that expects the phone should
list it in `BusServer(expected_peers=...)` so the question queues before the first
visit.

## Integrity

Capsule digests and the per-node hash chains detect accidental damage and
tampering **by anyone without the ability to rewrite the whole capsule**. They are
not signatures. When a capsule crosses a trust boundary, sign it:

    inari-core keygen me                      # me.key (keep private), me.pub
    inari-core sign capsule/ --key me.key
    inari-core verify capsule/ --trusted me.pub

The Ed25519 signature (extra `inari-core[signing]`) covers the bytes of
`manifest.json`, which digests every file and the journal, so any change breaks it.
It lives in `signature/`, outside the manifest's digests. It attests an exported
state: a capsule that keeps running changes its holder and journal and must be
signed again.

Archives are extracted with traversal protection; the workspace restore refuses
paths that leave the target directory.

## Not covered

Encryption of links (the TCP transport authenticates and signs every frame but does
not encrypt: run it over WireGuard, Tailscale or a TLS tunnel), and key rotation.
The container runtime drops capabilities and the network by default; it is only as
strong as the engine under it.
