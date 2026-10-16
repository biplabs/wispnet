# Registry storage foundation (#6)

`wipsnet_registry::store::Store` provides a local SQLite persistence boundary.
The `registry` CLI opens `registry.sqlite3` in the configured state directory,
checks integrity, and logs the current revision before waiting for shutdown.
The authenticated loopback API and HTTPS deployment are documented in
[registry API](registry-api.md). The [enrollment guide](enrollment.md) covers short-lived invitations, proof of
key possession and explicit approval.

The database belongs in a current-user-owned directory with mode 0700. It opens
only a regular, singly linked, current-user-owned file with mode 0600, refusing
final-component symlinks. The private directory must remain trusted; these checks
do not protect against another process running as the same user replacing paths.
SQLite is bundled for a reproducible native build. Android packaging is separate.

Schema version 3 uses SQLite `user_version`. Earlier schemas migrate transactionally;
unknown future versions fail closed. A new
database starts at revision 0. Devices use Iroh public endpoint IDs; services use
the tunnel protocol's bounded service IDs. Foreign keys require every service
host and grant recipient to exist in the device directory. A snapshot reads the
directory, hosting endpoints, grants, and revision in one transaction.

A replacement uses an immediate write transaction with an expected revision.
Stale writers receive `Conflict`; successful replacements increment the revision
once. A failed insertion rolls back deletions and the revision together. Limits
are 1,024 devices, 1,024 services, 1,024 grants per service and 16,384 grants total.
Errors contain static text, without database paths or submitted values.

## Backup and restore

Use `Store::backup(destination)` with a new destination filename in trusted
private storage. It reserves a 0600 file, uses SQLite's online backup API, and
syncs the resulting file. It refuses to overwrite an existing backup. A failed
backup may leave an incomplete file: remove it and retry with a new filename.
Keep backups private; they contain the directory and policy. The database stores
credential hashes; back up the raw owner credential file
separately. Backup creation syncs both the file and its parent directory.

For an offline restore, stop all registry processes, copy the backup into a new
private directory, retain mode 0600, and open it through `Store::open`. Check
`ready()` and the snapshot revision before selecting that directory for use.
Never replace a live database or copy only its main file while it is running.
The automated recovery test performs a transaction, online backup, close/reopen,
copy to a new restore location, then compares the complete restored snapshot.

Run the storage tests with:

```sh
cargo test -p wipsnet-registry --locked
```

Tests cover recovery, transaction rollback after a foreign-key violation, stale
writers on independent connections, future schema rejection, invalid service
IDs, and refusal of shared files, symlinks, and hard links. The API and native
process tests also cover owner/device authorization,
credential persistence and revocation. Public HTTPS deployment still requires
provisioning a domain and reverse proxy as described in the API guide.
