# Wispnet homepage

Buildless public homepage for `https://wispnet.app`. The account and public-access
dashboard is a separate service at `https://cmd.wispnet.app`. This page has no
account state, scripts, analytics, or external asset dependencies.

Preview from the repository root:

```sh
python3 -m http.server 18400 --bind 127.0.0.1 --directory apps/website
```

Deploy `index.html`, `style.css`, and `favicon.svg` together to a versioned folder
on the existing HTTPS gateway, with
`~/.local/share/wispnet-website/current` pointing to that folder. Merge the exact
apex-host block from `deploy/website/Caddyfile.fragment` into the existing Caddy
configuration, preserving relay, wildcard publications, and Command Center.
Validate Caddy before applying the configuration. The current gateway disables
its admin API, so applying it requires a service restart.

The Namecheap BasicDNS apex needs an `ALIAS` record: host `@`, value
`relay.wispnet.app`, TTL Automatic. Preserve existing subdomains and records.
Until this resolves, the public apex URL and its HTTPS certificate cannot be
verified. The existing gateway uses TLS-ALPN on port 443 for certificate issuance;
reuse that issuer configuration when merging the homepage block.

Client availability is described as a private preview because the repository is
private and no public releases currently exist. Update these links and copy when
public installers are deliberately released.
