# Optional desktop Command Center

`CommandCenterModule` owns desktop account linking, public publishing and the
host-local backend authentication editor. Private pairing, device grants,
discovery and service opening do not import or call this module.

`CommandCenterAccountButton` is a secondary entry inside the desktop gear
settings. It shows **Sign in with GitHub** or the linked account name,
and opens an account-only sheet with sign-in, public-access management and sign
out. Signing out confirms that account-linked public publishing will stop.
The control reads owner-local status only while settings are open and on account
changes. The home screen does not load account status. The
existing linking sheet polls only while open to complete browser approval.

The default command center is supplied by the node's local
`command_center_status` response. Browser launches use the desktop adapter's
`launchCommandCenter` operation, which resolves its destination from node state;
the Flutter UI cannot supply an arbitrary browser URL.

## Disable or remove

Build Flutter with `--dart-define=WISPNET_COMMAND_CENTER=false` to remove the
module's widgets and account API calls. Existing generic public link displays
remain ordinary service metadata. The flag is a UI choice; node authorization
must continue to protect any active publications.

To remove the integration entirely:

1. Remove the import and `CommandCenterAccountButton` entry in `app.dart`, and
   the import and `CommandCenterModule` entry in `desktop_settings.dart`.
2. Remove this directory and `test/command_center_test.dart`.
3. Remove the `command_center` import and dispatch hook from `desktop/bridge.py`,
   then remove `desktop/command_center.py` and its test.
4. Remove the helper's desktop bundle/install copy entries.

No change to the normal private device interface is required.

## Local node contract

All operations use the existing protected `desktopRequest` owner adapter.

- `command_center_status` is a local, non-networking read of account link,
  pending browser code and redacted credential status.
- `command_center_link` starts GitHub browser linking; `command_center_poll`
  completes it. The node keeps device/publisher credentials out of UI responses.
- `command_center_unlink` stops managed public publishing and unlinks the host;
  private grants remain independent.
- `configure_public` requires `managed: true` for new publications and includes
  the exact, case-sensitive `mcp_path` and service incarnation.
- `configure_backend_auth` accepts `passthrough`, `none`, or `bearer`. Credential
  status contains no token value, and the UI never requests a saved secret.

The backend token input is masked and cleared before sending the owner request.
An already configured token is never loaded back into an editor.
