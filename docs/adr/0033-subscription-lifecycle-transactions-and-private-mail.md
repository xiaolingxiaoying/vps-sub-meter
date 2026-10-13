# Subscription lifecycle transactions and private mail

The October 13 sbctl review found that named links advertised the default secret,
credential recovery captured an already-mutated list, and directory-wide ownership
preparation delegated SMTP secrets to the subscription daemon. Delivery mode changes
also saved configuration without changing the running service.

## Decision

- Authenticated overview and QR routes advertise the exact credential that
  authorized the request, including a retiring credential within its grace window.
  Default CLI URLs retain their default credential. Node artifacts remain shared.
- Credential operations hold the operation lock through reading, writing, restart,
  health checks and recovery. Recovery restores the complete original configuration;
  successful links are printed only after the running deployment commits.
- Revoking a name applies to every still-active credential for that name,
  including historical links retained by rotation. A revocation grace window
  never extends an earlier expiry; already-expired records stay unchanged.
- Storage preparation applies ownership only to known managed paths. SMTP config
  and delivery state remain `root:root 0600`. `/etc/sbctl` is `root:sbctl 0750`;
  `/var/lib/sbctl` is `root:sbctl 1771`. The sticky directory allows the daemon to
  atomically replace its own accounting files while preventing removal or replacement
  of root-owned mail state. Administrator overrides are not recursively delegated.
  Ownership/mode changes, including rollback metadata, use opened inode descriptors
  and reject symlinks in every path component, preventing service-controlled path
  replacements from redirecting a privileged chown/chmod to SMTP secrets.
- `config switch-mode` uses a lifecycle transaction for configuration, generated
  artifacts, active kernel configuration, managed unit files, affected pinned TLS
  files and the enabled/active states of the subscription, kernel and public socket.
  Failed checks or restarts restore the old deployment. Accounting and rotation
  oneshots may be waiting for the operation lock; delivery transactions do not alter
  their timer states or wait for those oneshots during recovery.
- The three modes are supported in both directions. Operators explicitly provide
  a new host/port when the old value cannot serve the destination mode. Moving from
  domain protocol certificates to IP fallback requires `--certificate-mode self-signed`.
  Existing credentials, protocol ports, accounting bytes and overrides are preserved.
- Direct mode requires free public TCP 80/443 when entered, and a valid pinned or
  live certificate. Otherwise it requires Certbot and an ACME email; it starts the
  HTTP challenge listener, obtains the certificate, and checks/restarts the final
  server before committing. Temporary protocol TLS is never advertised as insecure
  in domain client artifacts.
- External proxy HTTPS remains externally managed. Domain protocol TLS requires a
  valid existing live certificate; switching pins a private managed copy so the
  kernel can read it without changing the reverse proxy's files or their ownership.
  Refresh that copy by repeating `switch-mode --mode external-proxy` after renewal.
- Scheduled credential rotation is an opt-in root oneshot with narrowly writable
  configuration, state, managed unit and deploy-hook directories. Existing managed
  services are migrated without enabling a new timer or changing its schedule.
  Uninstall disables/stops and removes both optional timer pairs, checking ownership
  before changing any of their files.
- Overview usage uses the accounting report's corrected total. RX/TX and the
  standard subscription header's quota semantics stay unchanged. Grace parsing
  rejects invalid input and checked-arithmetic overflow before any configuration write.

## Compatibility and verification

The persisted deployment schema is unchanged. The new mode-switch options are
additive; successful mode changes now require a kernel check and a healthy running
deployment. Certbot's accounts and issuance history are not deleted during recovery.

Unit/CLI regressions and a real Linux UID/GID chroot check accompany the changes.
Production release acceptance requires the existing real-systemd suite on
Debian 12, Ubuntu 22.04 and Ubuntu 24.04; WSL checks alone do not satisfy that gate.
The Docker suites passed on all three distributions on 2026-10-13. Their scope
and the Linux permission fixes found during that run are recorded in
`docs/sbctl-review-fixes-2026-10-13.md`.
