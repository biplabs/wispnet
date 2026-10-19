# Device display names

Android uses the system device name, falling back to the model name when the
system setting is unavailable. Manage system → Device name saves an app-specific
override without changing the Android system setting. Names are limited to 128
UTF-8 bytes and cannot be blank or contain control characters.

Pairing sends the current name. For existing pairings, the bounded `set_name`
control RPC updates only the transport-authenticated caller's approved client
record. It accepts no caller-selected peer ID and cannot create an approval or
change grants, service incarnations, device keys, or saved browser addresses.
The host persists the label and advances its revision only when it changes.

Android records each host's acknowledgement separately. Refreshing services and
online background retries synchronize pending names; acknowledged names cause
no periodic network request. An unreachable, revoked, or older host never gets
marked synchronized. Browsing and catalog refresh remain independent of name
update support. Host apps predating this RPC must be updated to receive renames.

The desktop displays the saved client name with a short identity suffix so two
devices can use the same name. The full identity remains available in device
management. Names remain display metadata rather than identity or authority.
