# Simple setup, explicit recovery upgrades and optional shared TCP 443

Date: 2026-11-16

## Decision

The protocol catalog supplies human-readable names consisting of protocol and
transport names. Normalizing names and completing the nine-preset catalog are
explicit operations; updating the manager never silently enables extra nodes.
Existing IDs, credentials, ports, group membership and subscription URLs remain
stable. Per-format compatibility counts are shown separately from enabled nodes.

Fresh setup offers five protocols in one core by default, or all nine in dual
mode. Domain-less setup can discover a public egress IP over bounded HTTPS
requests that bypass proxy environment variables. It uses IP fallback HTTP on
2080, with an explicit warning about the private subscription URL. Reality has a
default decoy SNI; certificate protocols retain the established automatic SNI.
Public egress IPs are advertised but not bound as local addresses behind NAT.

The existing subscription HTTPS listener may inspect ClientHello SNI and relay
untouched TLS bytes to explicit loopback backends. This is the same L4 routing
principle documented by [NGINX ssl_preread](https://nginx.org/en/docs/stream/ngx_stream_ssl_preread_module.html).
It adds no daemon, traffic cache or access log. Parsing is bounded at 64 KiB / 5
seconds, with separate 256-global / 64-per-peer long-connection budgets. Backends
use a 3-second connection timeout; sessions have a 24-hour maximum lifetime.

This partially supersedes ADR-0007: managed nodes may also advertise 443. At 443,
TCP and UDP occupy separate namespaces. A TCP node in Direct mode must use an
explicit SNI route and listen on an unprivileged loopback backend. Managed core
units retain non-root identities and receive only CAP_NET_BIND_SERVICE for direct
UDP/standalone TCP 443. Other ports keep the established conservative namespace.
Same-SNI services cannot be separated by HTTP path at this layer. ECH-hidden SNI
and multiple QUIC services on UDP 443 are not routed. No foreign application
configuration is modified; its owner moves it to a loopback HTTPS backend first.

Official upstream remains every core download's default. This partially
supersedes ADR-0038's official-only rule: an administrator may explicitly provide
an HTTPS direct-file URL and SHA-256 for a validated core version. The UI explains
that a matching hash proves consistency, not authenticity. HTTPS redirects,
bounded size, archive member selection, executable identity, generated-config
validation and transactional activation still apply. Webpages/install scripts
are rejected. Alternate sources are never selected by fallback or by manifests.

The authenticated new binary drives recovery upgrades, without executing an old
CLI. Manager updates preserve data planes and sources; core upgrades separately
regenerate server configuration for the candidate version, avoiding removed 1.10
fields on modern sing-box. Failure restores owned affected files and unit states.
Unknown/corrupt schemas fail closed, with no reset or invented credentials.

SMTP delivery uses its own private lock and snapshots data under the deployment
lock before releasing that lock for network I/O. Passwords use a private temporary
curl config. Daily/reminder deduplication and last successful delivery are saved
only after successful SMTP delivery; subscription URLs are opt-in.
