# Typed protocol combinations and explicit candidate exports

The administrator expanded the multi-core work to include VLESS XHTTP Reality,
VLESS XHTTP TLS, VLESS WebSocket TLS, and Trojan TCP TLS, with export paths for
all client families. Represent these as validated presets over a common node
document, rather than an unrestricted protocol/transport/security cross-product.

`core::presets` owns the candidate node schema, cross-field validation, paired
Reality keys, Xray server/client renderers and compatible client node exports.
It does not change the persisted five-protocol Managed protocol registry yet.
`sbctl protocol list/check/export` exposes the candidate work without touching
services, deployment state, accounting or existing subscription artifacts.
This is a staged implementation boundary, not completion of managed activation.

Client formats are explicit: generic URI/Xray exports may carry XHTTP;
supported official sing-box, conservative Mihomo and Shadowrocket exports omit
unvalidated XHTTP nodes with a diagnostic, and fail if no compatible nodes remain.
An explicit experimental `mihomo-xhttp` export follows upstream's documented
schema; it does not imply compatibility with old Clash or every GUI version.
All GUI import and per-core data-path acceptance remains required.

This extends ADR-0006/0018's behavioral compatibility model and revisits the
Managed protocol scope, but leaves ADR-0007's activation transaction mandatory.
Do not add these candidates to the installation menu or subscription route
until canonical persistence, kernel selection, certificate permissions, version
gates and rollback are integrated. It does not supersede ADR-0002's subscription
credential or no-query-override boundary. Candidate generation does not justify
adopting third-party deployments, acquiring public ports or installing a proxy.

TLS clients verify certificates by default. Fixed Xray 26.10.31 no longer accepts
allowInsecure; test fixtures use an explicit disposable leaf certificate pin.
The local HTTP data-path test permits only a loopback test destination in a
test-specific freedom configuration. Production candidate exports keep the
upstream private-target protection. Secret-bearing diagnostics are not copied
to regular CLI output; explicit export intentionally prints credentials.

See `docs/protocol-presets.md` for invocation, limitations and evidence, and
`.scratch/runtime-templates-xray/issues/09-protocol-combinations.md` for the
remaining work before managed activation.
