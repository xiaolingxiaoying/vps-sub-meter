# Loopback clash API on the server for data-plane observation

`sbctl status` could tell an operator that `sing-box.service` was active and
nothing else: not its full version, not its memory, not what it was carrying.
The generated **client** profiles have enabled `experimental.clash_api` for a
long time, but the server configuration never did — it is `log` + `inbounds`
plus an IPv4 pin — so there was nothing to query.

## Decision

- `server_clash_api: Option<{ port, secret }>` in the deployment config, absent
  by default. Enabling it is explicit (`sbctl sing-box api enable`), because the
  clash API can enumerate *and terminate* live proxied connections; an artifact
  golden asserts the disabled path keeps the server bytes identical.
- The listener is always `127.0.0.1:<port>` with a generated 256-bit secret and
  a randomised high port. Nothing in sbctl forwards or proxies it, and the
  secret is stored in the 0600 config file rather than printed: `api status`
  shows six characters, and error strings are tested not to contain it, so a
  failed query cannot paste it into the journal.
- Observation reads it through a hand-rolled HTTP/1.1 GET over `TcpStream`
  (`observe::clash_api_request`) rather than a client crate: the transport is
  plaintext loopback, the CLI is synchronous, and the alternative would add a
  dependency and a TLS stack to read two JSON documents.
- `sbctl sing-box status` keeps working without the API: state, PID, restarts,
  cgroup memory and uptime come from `systemctl show` with a `/proc/<pid>/status`
  VmRSS fallback. The API adds *live* memory and `sbctl sing-box connections`;
  when it is off, that verb says which command turns it on rather than failing
  obscurely.
- Changing the server configuration goes through the existing
  `apply_config_transaction` + `restart_services_with_rollback` path, so a
  kernel that rejects `experimental.clash_api` (or a restart that does not come
  back) restores the previous deployment instead of leaving the proxy down.

## Consequences

- An operator who enables the endpoint accepts that any process running as root
  or as the `sing-box` user on that host can query and interrupt connections;
  the loopback bind and secret are there to keep everyone else out.
- The port and secret are stable across restarts, so a hand-rolled dashboard
  pointing at the port survives until the next `api enable`.
- `connections` output is a summary line per connection (destination, tallies,
  chain), not the raw API document, so the shape is ours to keep stable.
