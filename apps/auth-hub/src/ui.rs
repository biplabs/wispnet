//! Static Command Center UI. Identity and sharing policy stay in server modules.

pub const SCRIPT: &str = include_str!("../assets/command.js");
pub const CSS: &str = include_str!("../assets/command.css");

pub fn page(path: &str) -> String {
    let title = match path {
        "/connect" => "Connect a computer",
        "/consent" => "Authorize an MCP client",
        _ => "Command center",
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title} · Wispnet</title><link rel="stylesheet" href="/assets/command.css"><script defer src="/assets/command.js"></script></head><body><header><a class="brand" href="/">Wispnet<span>command center</span></a><div id="account"></div></header><main id="main"><p class="eyebrow">PUBLIC ACCESS</p><h1>{title}</h1><p id="status" role="status">Loading…</p><div id="content"></div></main><footer>Private device connections stay independent of your public account.</footer></body></html>"#
    )
}
