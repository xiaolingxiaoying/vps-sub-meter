# Publish companion presets through the existing subscriptions

The administrator explicitly requires the four additional combinations in the
live subscriptions without changing their URLs. This supersedes ADR-0039's
candidate-only boundary for the additional listener set. Retaining existing
TUIC/AnyTLS and adding Xray-specific XHTTP requires a companion data plane;
the earlier single-active-core rollout plan does not apply to this deployment.

The existing sing-box deployment keeps its credentials and listeners. A
non-root `sbctl-xray.service` owns the four new high TCP ports and reuses the
managed domain certificate. It is PartOf/Requires sing-box.service, so lifecycle
and certificate refresh restarts propagate; this was tested on the VPS.

`/etc/sbctl/protocol-presets.json` is the root-private persistent source for the
additional nodes. `sbctl protocol enable --input ...` requires an active owned
companion and exact agreement with its server inbounds before publication. It
holds the existing operation lock and regenerates the existing artifact set,
without rotating a subscription credential or rewriting the deployment config.
The enable command does not itself install or reconfigure the companion.

Client renderers add supported nodes and extend existing proxy-group members
before administrator overrides and rule-list processing. Generic URI/base64
URI carry all four. Official sing-box and Shadowrocket-compatible artifacts
carry WS/Trojan, current Mihomo additionally carries XHTTP TLS, and legacy
Mihomo omits XHTTP. Current Xray Reality is not labeled Mihomo-compatible.
Existing nodes and all subscription/QR/index paths retain their identity.
The former bare-node byte freeze is extended only when the administrator
explicitly enables these additional nodes; deployments without the source
file preserve their historical snapshots byte-for-byte.

The HTTP overview reads a generated share-link index, never the private source
file or Reality private key. Configuration/source validation errors abort
generation instead of publishing partial incompatible data. Normal regenerate
and future rule/override updates retain the additional nodes. Private server
configuration and the publication source are distinct; update both and verify
the runtime before enabling a changed source.

This completes the requested live subscription integration on the authorized
test VPS. It does not claim a generic single-core switch, a fresh-install menu,
or every GUI client's interoperability. The explicit deployment driver retains
private recovery points and verifies old URLs, old nodes, generated client
documents and live network forwarding before declaring success.
