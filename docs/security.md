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

## Integrity

Capsule digests and the per-node hash chains detect accidental damage and
tampering **by anyone without the ability to rewrite the whole capsule**. They are
not signatures. If capsules cross a trust boundary, add a detached signature over
`manifest.json`; the format leaves room for it (`manifest.json` is not itself
digested).

Archives are extracted with traversal protection; the workspace restore refuses
paths that leave the target directory.

## Not covered

Authentication and encryption of links (delegated to the private network or the
chosen bus), sandbox isolation (the runtime's job), and key rotation.
