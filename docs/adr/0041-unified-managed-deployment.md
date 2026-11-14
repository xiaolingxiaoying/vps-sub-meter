# Unified nodes, independent cores and generation publication

Date: 2026-11-14

The administrator approved unified node management, multiple instances of a
preset, node groups and sing-box/Xray single or simultaneous operation.

## Decision

`/etc/sbctl/nodes.json` is a versioned, root-private source containing stable node
IDs, client tags, display names, engine ownership, enabled state and ordered
groups. Its typed payloads reuse the established canonical and preset adapters.
Once present, legacy protocol credentials and protocol-presets.json are import
sources only. Unmigrated deployments retain their historical renderers.

Each enabled node belongs to exactly one selected engine. Both non-root services
use independent units; Xray no longer Requires/PartOf sing-box. Existing nodes
retain their engine and public parameters on migration. Mode changes migrate
supported nodes or require explicit disabling of incompatible nodes.

The operation lock covers expected-state comparison, validation, activation and
recovery. A private durable journal records affected owned files and service
states before activation. The HTTP service reads a committed immutable client
generation selected by one atomic pointer; it never reads the node source or
server configuration. Server documents/private keys are excluded from client
generations. Historical rollback excludes subscription credentials and accounting.

The client target version is pinned at migration, independent of engine updates.
Optional runtime client templates contain base settings; malformed existing
templates fail closed. Node and strategy definitions come from the registry,
followed by administrator overrides, rule lists and final reference validation.
All formats and the overview share one compatibility catalog.

Groups select content, not access rights. `/sub/<credential>/groups/<id>/<format>`
and the matching `qr/<format>` keep path-credential authorization and reject query
overrides. Display names can change without changing the group's URL or node tags.

## Supersession and compatibility

This supersedes the single-active-engine restriction in the November 9 runtime
spec and ADR-0039's candidate-only rollout boundary. It generalizes ADR-0040's
companion deployment while retaining existing subscription routes and proxy
credentials. It extends ADR-0022's compile-time template rule with explicit runtime
base settings, while keeping bundled defaults and bare/URI override boundaries.
ADR-0007, ADR-0033 and the existing managed-filesystem protection remain required.

Public release still requires the existing distribution/systemd gate and fixed
real-core compatibility/forwarding tests. Windows and WSL unit tests are not proof
of production service behavior.
