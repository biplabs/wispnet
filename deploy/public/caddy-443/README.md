# Caddy port 443 migration

This opt-in bundle migrates the existing HTTPS user service to a system service
running as the same user. Only CAP_NET_BIND_SERVICE is granted; NoNewPrivileges
and the existing filesystem restrictions remain enabled. Certificate storage and
internal relay/gateway ports remain unchanged. Caddy keeps 8443 during the router
transition and adds 443 on IPv4 and IPv6.

Copy Caddyfile and activate.sh into one directory on the relay host. Run from the
account owning the current Wispnet user services:

```sh
sudo sh /home/johnny/.local/state/wispnet-caddy-443-prepared/activate.sh
```

The script validates the config and unit, stores a root-owned rollback snapshot
under /var/backups/wispnet-caddy-443.*, disables the old user service, and enables
the system service. It tests verified TLS and the relay ping endpoint on IPv4
443/8443 and IPv6 443, restoring the old service if activation fails. If UFW is
active it adds TCP 443 while retaining existing rules. Other firewall managers
are operator-managed. A short restart interrupts existing relay connections.

After successful activation, change the router TCP rule from external 443 to
internal 8443 into external 443 to internal 443. Do not remove 8443 until public
HTTPS has been checked through the new rule. IPv6 router firewall allowances and
AAAA records are separate steps.

Manage the migrated Caddy using `sudo systemctl ... wispnet-public-https.service`.
The packet relay and public gateway remain user services. The old public gateway
installer refuses to overwrite a migrated Caddy; it requires adaptation before
it can deploy to this layout.

To roll back manually, use the exact backup directory printed by activation:

```sh
sudo systemctl disable --now wispnet-public-https.service
sudo rm /etc/systemd/system/wispnet-public-https.service
sudo cp -p /var/backups/wispnet-caddy-443.REPLACE/Caddyfile.old /home/johnny/.config/wispnet-public/Caddyfile
sudo systemctl daemon-reload
systemctl --user enable --now wispnet-public-https.service
```

If already changed, restore the router target to 8443 too. The TCP 443 UFW rule
can remain during rollback. The temporary compatibility listener should be
removed in a later change after the router transition is verified.
