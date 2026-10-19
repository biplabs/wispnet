# Terminal management

Run `wispnet` to open the interactive terminal app. On a headless host:

```sh
ssh -t desktop.home wispnet
```

Use `wispnet --config /absolute/path/host.toml` for another independent host
configuration. Existing scripting subcommands and `wispnet --qrcode` remain
available. Piped input/output requires those scripting commands; interactive
management requires a real terminal.

The TUI is built with Ratatui and Crossterm, compiled into the existing native
executable. It needs no Qt, display server, ncurses library, Python UI runtime,
or additional TUI installation. It uses the same owner-only local socket and
node identity as the desktop. It starts no second Iroh endpoint. The optional
combined owner-operations view reuses the already installed `wispnet-node`
Python 3 standard-library helper and its existing SSH owner routes and journals.
A normal local TUI session needs neither Python nor SSH.

## Navigation

- Use arrow keys and Enter to select a menu, then a row. Tab switches panes.
- Keys 1–8 open a view directly; `g` generates a QR and `p` opens pending approvals.
- Click a sidebar item to open it; double-click a row to review it. Confirmation
  choices and form controls also accept mouse clicks. Most terminal emulators
  allow text selection while holding Shift when mouse capture is enabled.
- `r` refreshes state, Esc returns/cancels, and `q` exits. Ctrl+C exits immediately.
- In forms, Tab/arrow keys change fields, Space toggles device/target checkboxes,
  and left/right arrows choose a protocol. Enter on **Review and save** opens a
  separate confirmation. Ctrl+U clears the current text field. Bracketed paste
  is supported; pasted invitation capabilities are masked in the form.
- Review dialogs support PageUp/PageDown for long details. Exit, errors and
  SIGTERM restore the cursor, raw-mode flags, mouse capture and alternate screen.

## Pair and approve

Select **Pair with QR**, or press `g`. The full host ID, QR and five-minute
countdown appear. Enlarge the terminal or reduce its font size if necessary;
the app refuses to wrap or clip a QR. QR contrast stays black on white even if
`NO_COLOR` is set. No capability is saved to terminal history or a file by the TUI;
the displayed QR remains sensitive until it expires or is consumed.

Scan using Wispnet on the phone. The terminal automatically switches to the
incoming request. Select the device, review its full ID, choose **Approve this
device**, then confirm. Rejection follows the same review flow. Only the exact
proved invitation is approved. Approval alone grants no services.

Next open **Local services**, choose a service, and select **Edit service and
permitted devices**. Toggle the intended devices and review/save. The same
form adds services, chooses HTTP/TCP, edits their loopback backend, and removes
a service. Replacing an existing service ID explicitly warns that the backend
and full access list will be replaced.

To pair this computer to a remote host, use **Pair remote host** and paste its
pairing code or import its bounded mode-0600 invitation file. Compare the full
host ID in the preview before confirming the request. Its owner must approve
this computer and grant services there. The TUI shares display names with the
desktop and clears that host's stale display catalog after explicit re-pairing.

## Machines and services

**Machines & devices** combines saved hosts and clients approved here, showing
identity, activity and available transport telemetry. Select a host to refresh
its authenticated catalog and browse permitted services. Catalog errors remain
explicit, and no stale cached catalog authorizes Open. HTTP services use the
same native browser gateway; TCP services use `wispnet tunnel`.

**Remove machine / revoke local access** shows the full target ID and asks for
confirmation. It revokes that client's grants on this node, ends active access,
and removes its saved host pin and browser routes. Permissions on the remote
host must be revoked there. No other machine's approval changes.

On a graphical session, Open launches the local browser. Over SSH it displays
the local gateway URL instead. That URL belongs to the remote computer; use a
browser there or explicitly forward the corresponding local gateway port and
resolve its `.localhost` name on the browser's computer. It is not a public URL.
Taking the node offline closes local browser routes; the relay remains separate.

## State and owner operations

Overview controls start/stop the installed node and enable/stop browser access.
Diagnostics shows the full host ID, revision, relay state and active transport
count. Network work runs off the event loop. Confirmed actions that arrive during
a refresh wait for that operation and retain their reviewed revision; conflicts
are reported without automatic mutation retries. Offline state cannot be used
to mutate permissions. Exiting during an in-flight request can leave its result
unconfirmed; refresh before making another decision.

**Owner operations** offers the desktop's explicit per-target add/revoke workflow,
with unchecked target checkboxes, full IDs, durable independent acknowledgements,
and retry of pending targets only. Configured targets come from the protected
`admin-targets.json` alongside the host config. Existing invalid/unreadable target
files are reported rather than silently substituting another host. Each target
uses its own owner authority; an SSH route must already be trusted. This never
turns an ordinary peer pairing into remote administration.

## Validation

Local Rust tests cover pinned approval decisions, stale revisions, cancellations,
explicit grant lists, private paste, QR expiry/size, queued actions, socket
permissions and API versions. The real PTY integration test runs against isolated
native hosts and exercises approval/cancellation, service grants, machine removal,
invitation-file pairing, mouse navigation, stale-conflict rejection, terminal
resize and restoration on Ctrl+C/SIGTERM. It independently decodes the QR from
actual terminal cells. These checks do not replace live optical camera scanning.
