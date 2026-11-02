# Official-only kernel downloads and opt-in Hy2 port hopping

Supersedes the repository-kernel source choice in ADR-0032. Per the administrator's request, every network sing-box download now resolves the SagerNet official archive from version and architecture; no repository mirror is offered or used. Offline local candidates remain supported. New Releases publish sbctl, signed manifests, installer and checksums, without re-hosting sing-box binaries.

Extends ADR-0010 and ADR-0024: a signed manifest still pins the extracted executable's SHA-256 and exact version. Official archive download additionally requires GitHub's archive digest. Manifest kernel URLs are informational; download code never uses them. The release signer verifies pinned official archive bytes before recording the executable digest. Old signed manifests remain readable, but their kernel download URLs cannot select an alternative source.

Extends ADR-0007/0012: optional Hy2 port hopping stores a bounded UDP range and interval, validates conflicts, and uses the existing configuration/activation rollback transaction. A root systemd pre/post hook manages only the reserved inet sbctl_hy2 nftables table. The data-plane process retains its non-root identity. Disabled deployments retain the historical unit without these hooks. Disabling reconciles the table before removing the hooks; stopping/uninstalling an enabled service runs cleanup.

Client fields follow upstream support: 1.10 keeps the original fixed port; 1.11+ receives server_ports and hop_interval; Mihomo receives ports and hop-interval. Official multi-port URIs use client-default intervals because the URI specification defines no interval field. All share links and subscriptions derive from the same persisted setting.

Finite confirmation choices accept 1/2 as well as legacy text. The TLS prompt explains that VMess and the other certificate protocols share generated/self-signed or managed domain certificates; selecting automatic SNI clears an old custom override.
