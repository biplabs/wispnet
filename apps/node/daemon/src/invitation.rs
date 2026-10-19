//! Shared owner-local QR issuance for the terminal and desktop. Never log capabilities.
use qrcode::{Color, QrCode};
use wispnet_core::{config::Config, identity::IdentityStore};

pub fn issue(config: &Config) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if config.schema_version != 2 {
        return Err("QR pairing requires an independent host configuration".into());
    }
    let identity = wispnet_agent::identity::FileIdentityStore::new(&config.state_dir)?.load()?;
    let relay = config
        .transport
        .as_ref()
        .and_then(|t| t.relay_url.as_deref())
        .ok_or("Pairing requires a configured relay")?;
    let machine = std::process::Command::new("hostname").output()?;
    let name = std::str::from_utf8(&machine.stdout)?.trim();
    if !machine.status.success() || !wispnet_core::enrollment::valid_name(name) {
        return Err("Cannot read a valid machine name".into());
    }
    let (mut store, secret) =
        wispnet_registry::host::owner_store(&config.state_dir, identity.endpoint_id())?;
    let mut invitation = store.pairing_invite(
        identity.endpoint_id(),
        &secret,
        relay,
        wispnet_registry::enrollment::now(),
        300,
    )?;
    invitation.host_name = Some(name.to_owned());
    let code = wispnet_agent::pairing::encode_invitation(&invitation)?;
    let qr = QrCode::new(code.as_bytes())?;
    let width = qr.width();
    // Include the standard four-module white quiet zone in both renderers.
    let mut matrix = vec![vec![false; width + 8]; width + 8];
    for y in 0..width {
        for x in 0..width {
            matrix[y + 4][x + 4] = qr[(x, y)] == Color::Dark;
        }
    }
    Ok(serde_json::json!({"host":invitation.host,"host_name":name,
        "invitation":invitation.invitation,"expires_at":invitation.expires_at,"matrix":matrix}))
}

pub fn terminal(matrix: &[Vec<bool>]) -> String {
    let mut output = String::new();
    for rows in matrix.chunks(2) {
        // Explicit black-on-white works with both light and dark terminal themes.
        output.push_str("\x1b[30;107m");
        for (x, top) in rows[0].iter().enumerate() {
            let bottom = rows.get(1).is_some_and(|row| row[x]);
            output.push(match (*top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        output.push_str("\x1b[0m\n");
    }
    output
}

pub fn terminal_columns() -> Option<usize> {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // Read the output terminal only; no terminal settings are changed.
    if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0
        && size.ws_col > 0
    {
        Some(usize::from(size.ws_col))
    } else {
        None
    }
}
