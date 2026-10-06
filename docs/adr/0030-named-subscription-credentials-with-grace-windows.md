# Named subscription credentials with grace windows

`sbctl credential rotate` replaces the single deployment-wide path secret, so
revisting a leaked link means cutting off every device at once. Operators
sharing one server with family and themselves on several devices need to retire
one link without touching the rest.

## Decision

- `subscription_credential` stays the default link (so every existing client,
  URL builder and test keeps working unchanged), and
  `subscription_credentials: Vec<NamedCredential>` holds additional links, each
  `{ name, credential, revoked_at }`. `#[serde(default)]` means a config written
  by an older sbctl loads as-is: the feature is additive, not a migration.
- A record's `revoked_at` **is its expiry**. `revoke` without `--grace` writes
  the current instant (not `None` — `None` means "never expires", so writing it
  would have kept serving the link the operator just retired). `--grace 30m`
  writes `now + 1800`.
- `rotate --name X --grace W` keeps the promise by **remembering the retiring
  secret as its own record**: the live record takes the fresh value, a second
  record with the previous secret expires on its own. Overwriting in place would
  make `--grace` a claim nothing holds.
- Validation allows one *live* record per name (expiry-free); additional
  same-name records must carry a future expiry. Two live records would make
  `rotate --name` ambiguous.
- Authentication compares every candidate — live, retiring and expired — with the
  existing constant-time helper, then applies the window. `is_active() && eq()`
  would skip the comparison for an expired entry and hand back a timing signal
  for "this name exists". The uniform 404 for unknown, expired and valid-but-
  wrong links is only meaningful if the check behind it is uniform too.
- `credential list` masks every secret to its first six characters; `sbctl sub`
  remains the only command that prints a full path secret, to this terminal.
- Nothing here changes artifact content, so add/revoke/rotate restart the daemon
  (which reads the accepted set at startup) instead of regenerating artifacts.

## Consequences

- ADR-0021 refused per-request *content* rewriting; this is multiplicity at the
  authentication layer — every accepted credential still receives the identical
  artifact bytes.
- Rotating the **default** credential remains all-or-nothing by design: it is
  what `sbctl sub`, the index page and the QR routes advertise, so a grace window
  on it would mean advertising a link that is already being retired.
- Expired records are harmless but accumulate; `credential add` reuses an
  expired same-name record instead of appending a third, and the list view shows
  each record's state so an operator can see what to clean up.
