# Subscription content templates are a compile-time catalog

Client artifacts were thin: one proxy group, no routing verdicts beyond what a
bare profile needs, no sniffing configuration, and rule sets that vanished
wholesale under `client_rule_profile = "minimal"`. Richer content is a product
decision per *client generation*, so it is expressed as a template axis:

```text
canonical nodes → template (built-in) → client_rule_profile (CDN or not) → overrides (two administrator files) → artifact
```

`ClientTemplate { Standard, Global, Split }` lives in `src/subscription/template.rs`
as a compiled-in `TemplateSpec` (groups, rule sets, inline rule twins, DNS,
sniffing, final group). It is not a directory of files on disk, and it does not
reuse the `ClientRuleProfile` axis.

**`Standard` reproduces today's output byte-for-byte.** The artifact goldens in
`src/subscription/snapshots/` are the proof: introducing the axis must move zero
bytes, and any golden that moves is a deliberate product decision recorded with
an ADR amendment, not an implementation detail (ADR-0021).

Why compile-time rather than administrator-editable templates:

- a single template file cannot be valid for five sing-box minor versions at
  once, and ADR-0021 makes a malformed override abort regeneration. Putting the
  *default* content behind that rule would let one bad paste take down the
  default subscription for every client;
- the actual request behind the gap is "the artifacts are too thin", not "the
  templates must be editable by an administrator".

Why a separate axis rather than folding richness into `client_rule_profile`:

- `minimal` today means *delete the whole external-resource class* — no CDN rule
  sets, and therefore no split routing either. That is the regression this
  phase exists to fix. `minimal` keeps exactly one meaning, "never contact a
  rule CDN", and every
  template rule set ships an inline rule twin rendered from a compiled-in
  domain/IP list, so routing survives without a CDN.

Layering order is fixed and load-bearing: the template decides *what content
exists*, `client_rule_profile` decides *whether external resources are reachable*,
and administrator overrides are applied last so an operator can add or prepend
without editing generated structure. `client_rule_set_base_url` remains the only
CDN knob.

Out of scope: administrator-supplied template files, per-request template
selection (the subscription API still rejects query parameters, ADR-0002), and
any change to the bare `sing-box.json` / `uri` / `uri.txt` / `shadowrocket.txt`
artifacts, whose bytes are frozen. Enabling `--client-template split` may grow
groups, rule sets and `sniffers` in the full profiles; those four artifacts must
stay byte-identical, which is asserted by the goldens.

## Amendment — 2026-10-16: vendored client defaults

At the administrator's request, full client artifacts now take their baseline
settings from the mihomo-test `mihomo.yaml` and `sing-box.json` vendored under
`src/subscription/templates/` and embedded at compile time. This deliberately
supersedes Standard's historical byte-for-byte full-profile guarantee above;
the updated snapshots record the new product defaults. Bare node and URI
artifacts retain their previous contract.

The catalog still owns policy groups, routing modes and configurable rule-set
sources. Renderers adapt the baseline to those group names, DNS mode and
per-minor compatibility, keeping administrator overrides last. Template files
are repository inputs, not deployment-time administrator files. Portal DNS and
routing, TUN MTU 1400, the sample campus subnet exclusion and the mixed listener
now follow the supplied templates. See `src/subscription/templates/README.md`
for the precise adaptation boundary.

## Amendment — 2026-10-16: preserve the supplied standard template

The administrator explicitly requires all supplied template settings to survive.
Standard full sing-box profiles now use the complete vendored document, adapting
only live nodes, explicit administrator options and per-minor compatibility.
This supersedes the earlier amendment's DNS pruning and historical group names.
The standard resolver tags, DIRECT outbound and selector name follow the source.
Default sing-box rule-set URLs retain the supplied SagerNet sources; explicitly
configured mirrors still override them. Bare node/URI bytes remain unchanged.

All Mihomo artifacts use exactly the supplied manual and automatic groups,
regardless of the optional sing-box policy catalog. No custom GLOBAL or direct
group is emitted: the Mihomo core owns GLOBAL. The manual group lists nodes,
automatic selection and DIRECT in that order. This deliberately supersedes
Mihomo purpose-group catalogs; sing-box global/split remain available.

## Amendment — 2026-10-17: reviewed Mihomo template

The administrator supplied `mihomo-reviewed.template.yaml` as the new baseline.
It replaces the vendored Mihomo document verbatim and supersedes the campus
subnet exclusion, non-strict TUN, geo-based DNS policy and old sniff settings.
Generated Mihomo preserves its complete routing order and four review-prefixed
providers; minimal and legacy output substitute inline twins in both routing
and DNS. The standard sing-box baseline follows the same private-system / CN-DoH /
proxied-DoH policy, Fake-IP exclusions, explicit proxy exceptions, strict TUN
and local listeners. The new snapshots deliberately replace full-profile defaults;
bare nodes and URI artifacts retain their contract. Version limitations and DNS
upstream selection differences are documented in the template README.

## Amendment — 2026-10-17: compact and standard client choices

The interactive template selector offers two profiles:

- `compact` emits one manual proxy group containing the real nodes and
  `DIRECT`; it has no automatic-selection group.
- `standard` retains the reviewed manual and automatic group arrangement.

This is a group-layout choice only. Both profiles retain the reviewed DNS, TUN,
sniffing, routing rules, and rule-set behavior. The independent
`client_rule_profile` still selects remote rule sets (`standard`) or their
compiled-in twins (`minimal`); neither template choice removes those reviewed
policies. The sing-box 1.10–1.14 adapters remain responsible for each core's
DNS, AnyTLS, cache, and TUN-stack compatibility.

`global` and `split` remain valid serialized configuration values and
non-interactive `sbctl config template` inputs for existing deployments and
scripts. They are legacy choices hidden from the interactive template
selector; an explicit CLI request or a persisted legacy value continues to
render its existing policy. New menu choices are limited to `compact` and
`standard`.

The Standard sing-box artifact follows the reviewed baseline recorded above;
the compact profile changes only its selector membership and omits the
automatic group. The Mihomo standard profile keeps both supplied groups, while
compact omits `自动选择` and removes it from `代理` membership. Bare node and
URI artifacts remain byte-stable for every template.
