# Readable device and service IDs

Devices and services each have one readable ID, used in the UI and in generated
browser addresses. There are no separate display names or per-client aliases.
IDs use 1–64 lowercase ASCII letters, digits, dots, underscores or hyphens and
start with a letter or digit. A service ID is unique within its owning host.
Device IDs need not be globally unique; generated addresses retain the host-ID
suffix for disambiguation.

Device settings on desktop and Android label the editable value **Device ID**.
The protected Ed25519 endpoint identity is labeled **Host ID**. Changing a
readable device ID does not regenerate the host key, re-pair devices or alter
service grants. Services are displayed by their existing service IDs; those IDs
remain fixed when editing their backend, protocol or access list.

Android saves the OS device name/model as a readable ID on first use. Existing
app-specific names are normalized once into that same saved ID. Desktop accepts
`device_id` in node configuration; `device_name` remains a legacy alias for the
same field. Legacy device labels are normalized without moving the state directory
or changing any cryptographic identity. Set `device_id` explicitly on desktop
to keep the value independent of later OS name changes.

Pairing sends the current readable ID. The existing authenticated `set_name`
control RPC synchronizes it to approved hosts, updating only the caller's
existing record. Android records each host's acknowledgement separately and
retries pending updates while online. Desktop synchronizes during catalog refresh.
The bounded `device_id` control RPC returns the owning host's ID to approved,
identity-pinned peers; it updates cached names without creating a pairing or grant.
An older host may reject this optional RPC without invalidating its catalog.

SQLite schema 11 removes stored service display names. It preserves service IDs,
incarnations, host authority generation, owner credentials, pairings and grants.
When a catalog's old display name differs from its ID, the migration advances its
revision so existing client baselines can accept the changed response. Version-1
catalogs retain `display_name` solely for installed-client compatibility, always
derived from the service ID. Legacy owner requests may still send a bounded
`display_name`, but it is not stored or used by the UI.
