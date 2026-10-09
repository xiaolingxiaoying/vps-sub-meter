# Menu-managed kernels, custom server listeners and opt-in email

The bootstrap now installs only the authenticated sbctl binary and opens its menu.
Explicit install flags retain the unattended deployment transaction. Kernel source
selection occurs in the deployment menu: repository Release uses the verified
manifest's fixed binary; official downloads accept an exact version or latest
stable and retain the mandatory release digest and real-kernel checks.

This extends ADR-0029: server inbounds and outbounds merge by tag, including across
drop-in layers, so adding a listener does not replace generated nodes. Every
inbound override needs a distinct nonempty tag. Credential edits remain forbidden
on managed nodes; `custom-*` listeners may specify their own credentials. These
listeners are operator-owned and are not automatically advertised in subscriptions.
Successful menu edits validate the core and restart through the existing rollback
transaction; failure restores the override source and running artifacts. The
shared merge engine's default semantics remain unchanged.

Domain TLS installation stages only the server with a self-signed certificate so
the ACME HTTP listener can start before the issued files exist. Generated client
subscriptions keep domain verification enabled. Once ACME succeeds, the server
is regenerated and restarted with the validated pinned full chain. Failed issuance
rolls back the domain install. A domain protocol SNI must equal the subscription
domain, which is the name certified by this installation's ACME request.

Clash groups use one configured probe URL and non-lazy health checks, including
select and direct groups. CMFA queries provider health checks and per-URL delay
histories; inconsistent URLs or a group without a check made its displayed
latency unreliable. The default for new deployments is HTTPS gstatic generate_204;
existing explicit probe URLs are preserved and now apply to Clash too.

Email is separate, opt-in, root-only configuration. SMTP always uses verified TLS
(implicit TLS or mandatory STARTTLS). curl options and the MIME payload are private
temporary files; passwords never enter argv or error output. Reports read existing
VPS accounting, include service/kernel state, RX/TX, allowance and both reset
timezones. Subscription links require an explicit setting. An hourly systemd task
sends once per display-timezone day and once per next reset instant in a configured
advance window; state is saved only after SMTP success under the operation lock.
Missed pre-reset reminders are not retroactively sent after a period changes.
Uninstall removes the email timer; normal data backup preserves email settings.

Sources for the CMFA behavior:
- https://github.com/MetaCubeX/ClashMetaForAndroid/blob/main/core/src/main/golang/native/tunnel/connectivity.go
- https://github.com/MetaCubeX/ClashMetaForAndroid/blob/main/core/src/main/golang/native/tunnel/proxies.go
- https://wiki.metacubex.one/en/config/proxy-groups/url-test/
