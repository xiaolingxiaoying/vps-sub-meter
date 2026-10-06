# Layered overrides, a server-side target, and operator rule lists

ADR-0021 gave sbctl one override pair (`sing-box-override.json`,
`clash-override.yaml`) merged into the client artifacts. Operators needed three
things that pair cannot express: a place to extend the configuration **this host
runs**, more than one authored fragment per target, and a way to add routing
decisions without restating generated ones.

## Decision

- **Layering.** Each target is one base document plus a drop-in directory
  (`sing-box.d/*.json`, `clash.d/*.yaml`), merged in filename order after the
  base. A malformed layer aborts the whole regeneration so the transactional
  writer keeps serving the previous known-good artifacts.
- **A server target.** `sing-box-server.json` + `sing-box-server.d/` merge into
  `sing-box-server.json` before the real-kernel check, inside the existing
  apply/rollback transaction. This is the VPS-side extension seam ADR-0021
  deliberately left out.
- **Protected fields.** A server override that names an inbound credential field
  (`users`, `uuid`, `password`, `private_key`, `short_id`, `certificate`, …)
  is refused. Client overrides carry no such rule, because adding one's own node
  legitimately requires its credentials.
- **Merge policy is opt-in.** `json-merge` keeps `deep_merge` as the frozen
  ADR-0021 contract (objects recursive, arrays replace, `rules` prepended) and
  adds `deep_merge_with(base, overlay, MergePolicy)` for
  `rules_mode = append|replace` and identifier-keyed arrays (`outbounds` by
  `tag`, `proxies`/`proxy-groups` by `name`). The default path is asserted to
  reproduce every row of the shared `MERGE_SEMANTICS` table, which `client-core`
  also asserts, so widening the server's vocabulary cannot fork what "覆写"
  means for installed clients.
- **Rule lists.** `etc/sbctl/rules/{direct,proxy,reject,fakeip-filter}.list`
  take the `TYPE,value[,policy]` line format shared rule collections publish.
  They are injected as generated rules — ahead of the generated verdicts, reject
  first — and fake-ip exceptions join the generated DNS rule in place rather
  than forking it, which keeps the behaviour identical across sing-box 1.10–1.14
  and both clash shapes.
- **Two ways to bring outside lists in.** `sbctl rule-set add` registers a
  *compiled* rule-set URL (`.srs`/`.mrs`, https only) that each subscriber's own
  core downloads, so the server never becomes a bandwidth proxy for other
  people's rules and a plain-http mirror cannot downgrade a subscription's
  integrity. `sbctl rule-set import-list --url … --into …` is the other branch:
  it fetches a text list here, size-capped and never piped to a shell, and appends
  validated lines to the operator list above — for sources that only publish
  `.list` files, or when the operator wants the verdicts baked into the artifact
  instead of resolved on the device.
- **The bytes that ship are the bytes that were checked.** When any override or
  rule list is present, regeneration additionally runs the merged full client
  profile through a real `sing-box check`. The per-version probe that picks the
  profile runs before merging, so without this the customized artifact — the
  only one anyone actually receives — would ship unchecked.

## Consequences

- Group tags stay compile-time constants; a rule list can point traffic at a
  group, but adding a *new* group means a client override with keyed
  `outbounds`/`proxy-groups`, not a new name in a list file.
- `rules_mode` is a control key read from the operator's document and stripped
  before merging, so it never reaches sing-box or mihomo as an unknown field.
- A rule list typo fails generation loudly rather than serving half a policy;
  the same validation runs in `sbctl rule add` before anything is written.
