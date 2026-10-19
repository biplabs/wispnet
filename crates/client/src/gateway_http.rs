//! Deliberately bounded HTTP/1.1 compatibility boundary for browser routes.
use super::Request;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Clone, Copy)]
pub(super) enum Body {
    None,
    Length(u64),
    Chunked,
    Close,
    WebSocket,
}
pub(super) struct Head {
    pub bytes: Vec<u8>,
    pub body: Body,
}
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
fn length(s: &str) -> Result<u64, ()> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    s.parse().map_err(|_| ())
}
pub(super) fn connection(value: Option<&str>) -> Result<Vec<String>, ()> {
    value
        .unwrap_or("")
        .split(',')
        .filter(|v| !v.is_empty())
        .map(|v| {
            let v = v.trim();
            if !token(v) {
                return Err(());
            }
            Ok(v.to_ascii_lowercase())
        })
        .collect()
}
pub(super) fn cookies(request: &Request) -> Result<Vec<(&str, &str)>, ()> {
    request
        .values("cookie")
        .into_iter()
        .flat_map(|v| v.split(';'))
        .map(|c| {
            let (name, value) = c.trim().split_once('=').ok_or(())?;
            if !token(name) || !cookie_value(value) {
                return Err(());
            }
            Ok((name, value))
        })
        .collect()
}
fn cookie_value(value: &str) -> bool {
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    value.bytes().all(|b| {
        b == 0x21
            || (0x23..=0x2b).contains(&b)
            || (0x2d..=0x3a).contains(&b)
            || (0x3c..=0x5b).contains(&b)
            || (0x5d..=0x7e).contains(&b)
    })
}
pub(super) fn framing(request: &Request) -> Result<(bool, u64), ()> {
    for key in [
        "content-length",
        "connection",
        "upgrade",
        "sec-websocket-key",
        "sec-websocket-version",
        "sec-websocket-protocol",
    ] {
        if request.values(key).len() > 1 {
            return Err(());
        }
    }
    for key in ["transfer-encoding", "expect", "trailer", "te"] {
        if !request.values(key).is_empty() {
            return Err(());
        }
    }
    let n = request
        .one("content-length")
        .map(length)
        .transpose()?
        .unwrap_or(0);
    if ["GET", "HEAD"].contains(&request.method.as_str()) && n != 0 {
        return Err(());
    }
    let connection = connection(request.one("connection"))?;
    if connection
        .iter()
        .any(|v| !matches!(v.as_str(), "close" | "keep-alive" | "upgrade"))
    {
        return Err(());
    }
    let websocket = match request.one("upgrade") {
        None => {
            if connection.iter().any(|v| v == "upgrade") {
                return Err(());
            }
            false
        }
        Some(v) => {
            if !v.eq_ignore_ascii_case("websocket")
                || request.method != "GET"
                || n != 0
                || !connection.iter().any(|v| v == "upgrade")
                || request.one("sec-websocket-version") != Some("13")
            {
                return Err(());
            }
            if STANDARD
                .decode(request.one("sec-websocket-key").ok_or(())?)
                .map_err(|_| ())?
                .len()
                != 16
            {
                return Err(());
            }
            true
        }
    };
    Ok((websocket, n))
}
pub(super) fn clean_cookie(value: &str) -> Result<Option<String>, ()> {
    let mut parts = value.split(';');
    let first = parts.next().ok_or(())?.trim();
    let (name, value) = first.split_once('=').ok_or(())?;
    if !token(name) || !cookie_value(value) {
        return Err(());
    }
    if name == super::COOKIE {
        return Ok(None);
    }
    if ["__Host-", "__Secure-", "__Http-"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return Err(());
    }
    let mut attributes = vec![first.to_owned()];
    for part in parts {
        let part = part.trim();
        let (key, value) = part
            .split_once('=')
            .map_or((part, None), |(k, v)| (k, Some(v)));
        if !token(key) {
            return Err(());
        }
        if key.eq_ignore_ascii_case("secure")
            || key.eq_ignore_ascii_case("partitioned")
            || key.eq_ignore_ascii_case("samesite")
                && value.is_some_and(|v| v.eq_ignore_ascii_case("none"))
        {
            return Err(());
        }
        if key.eq_ignore_ascii_case("domain") {
            continue;
        }
        attributes.push(part.to_owned());
    }
    Ok(Some(attributes.join("; ")))
}
fn redirect(
    value: &str,
    origin: &str,
    binding: Option<std::net::SocketAddr>,
) -> Result<String, ()> {
    if value.contains('\\') || value.chars().any(|c| c.is_ascii_control()) {
        return Err(());
    }
    let base = url::Url::parse(&format!("{origin}/")).map_err(|_| ())?;
    let target = base.join(value).map_err(|_| ())?;
    if !target.username().is_empty() || target.password().is_some() {
        return Err(());
    }
    if target.origin() == base.origin() {
        return Ok(target.to_string());
    }
    if let Some(binding) = binding {
        let backend = url::Url::parse(&format!("http://{binding}/")).map_err(|_| ())?;
        if target.origin() == backend.origin() {
            return Ok(format!("{origin}{}", &target[url::Position::BeforePath..]));
        }
    }
    Err(())
}
pub(super) fn response(
    bytes: &[u8],
    request: &Request,
    origin: &str,
    binding: Option<std::net::SocketAddr>,
    websocket: bool,
) -> Result<Head, ()> {
    let mut storage = [httparse::EMPTY_HEADER; 96];
    let mut head = httparse::Response::new(&mut storage);
    if !head.parse(bytes).map_err(|_| ())?.is_complete() || !matches!(head.version, Some(0 | 1)) {
        return Err(());
    }
    let status = head.code.ok_or(())?;
    if !(100..=599).contains(&status) {
        return Err(());
    }
    let fields: Vec<_> = head
        .headers
        .iter()
        .map(|h| {
            Ok((
                h.name.to_ascii_lowercase(),
                std::str::from_utf8(h.value).map_err(|_| ())?.trim(),
            ))
        })
        .collect::<Result<_, ()>>()?;
    let values = |key: &str| {
        fields
            .iter()
            .filter(|(n, _)| n == key)
            .map(|(_, v)| *v)
            .collect::<Vec<_>>()
    };
    for key in [
        "content-length",
        "transfer-encoding",
        "connection",
        "upgrade",
        "location",
        "sec-websocket-accept",
        "sec-websocket-protocol",
    ] {
        if values(key).len() > 1 {
            return Err(());
        }
    }
    if !values("trailer").is_empty() || !values("refresh").is_empty() {
        return Err(());
    }
    let connection = connection(values("connection").first().copied())?;
    if connection.iter().any(|s| {
        matches!(
            s.as_str(),
            "content-length" | "transfer-encoding" | "location" | "set-cookie"
        )
    }) {
        return Err(());
    }
    let content_length = values("content-length")
        .first()
        .map(|s| length(s))
        .transpose()?;
    let chunked = match values("transfer-encoding").first() {
        None => false,
        Some(v) if v.eq_ignore_ascii_case("chunked") => true,
        _ => return Err(()),
    };
    if chunked && content_length.is_some() {
        return Err(());
    }
    if status != 101 && (!values("upgrade").is_empty() || connection.iter().any(|v| v == "upgrade"))
    {
        return Err(());
    }
    let body = if status == 101 {
        if !websocket
            || content_length.is_some_and(|n| n != 0)
            || chunked
            || values("upgrade")
                .first()
                .is_none_or(|v| !v.eq_ignore_ascii_case("websocket"))
            || !connection.iter().any(|v| v == "upgrade")
        {
            return Err(());
        }
        let mut hash = Sha1::new();
        hash.update(request.one("sec-websocket-key").ok_or(())?.as_bytes());
        hash.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
        if values("sec-websocket-accept") != [STANDARD.encode(hash.finalize())] {
            return Err(());
        }
        if let Some(protocol) = values("sec-websocket-protocol").first()
            && (!token(protocol)
                || request
                    .one("sec-websocket-protocol")
                    .is_none_or(|v| !v.split(',').any(|s| s.trim() == *protocol)))
        {
            return Err(());
        }
        Body::WebSocket
    } else if status < 200 {
        return Err(());
    } else if request.method == "HEAD" || status == 204 || status == 304 {
        if status == 204 && (chunked || content_length.is_some_and(|n| n != 0)) {
            return Err(());
        }
        Body::None
    } else if chunked {
        Body::Chunked
    } else if let Some(n) = content_length {
        Body::Length(n)
    } else {
        Body::Close
    };
    let mut output = format!("HTTP/1.1 {status} {}\r\n", head.reason.unwrap_or(""));
    for (name, value) in &fields {
        if matches!(
            name.as_str(),
            "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "proxy-connection"
                | "te"
                | "trailer"
                | "cache-control"
                | "pragma"
                | "expires"
                | "referrer-policy"
                | "x-frame-options"
                | "cross-origin-resource-policy"
                | "cross-origin-opener-policy"
                | "service-worker-allowed"
        ) || name.starts_with("access-control-")
            || connection.iter().any(|v| v == name) && name != "upgrade"
        {
            continue;
        }
        if name == "set-cookie" {
            if let Some(value) = clean_cookie(value)? {
                output.push_str(&format!("Set-Cookie: {value}\r\n"));
            }
        } else if name == "location" {
            output.push_str(&format!(
                "Location: {}\r\n",
                redirect(value, origin, binding)?
            ));
        } else {
            output.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    output.push_str(if matches!(body, Body::WebSocket) {
        "Connection: Upgrade\r\n"
    } else {
        "Connection: close\r\n"
    });
    output.push_str("Cache-Control: no-store\r\nPragma: no-cache\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: frame-ancestors 'none'\r\nX-Frame-Options: DENY\r\nCross-Origin-Resource-Policy: same-origin\r\nCross-Origin-Opener-Policy: same-origin\r\n\r\n");
    Ok(Head {
        bytes: output.into_bytes(),
        body,
    })
}
pub(super) async fn body(
    input: &mut (impl AsyncRead + Unpin),
    output: &mut (impl AsyncWrite + Unpin),
    body: Body,
) -> Result<(), ()> {
    match body {
        Body::None => {}
        Body::Length(n) => {
            if tokio::io::copy(&mut input.take(n), output)
                .await
                .map_err(|_| ())?
                != n
            {
                return Err(());
            }
        }
        Body::Close => {
            tokio::io::copy(input, output).await.map_err(|_| ())?;
        }
        Body::WebSocket => return Err(()),
        Body::Chunked => loop {
            let mut line = Vec::new();
            while !line.ends_with(b"\r\n") {
                if line.len() >= 128 {
                    return Err(());
                }
                line.push(input.read_u8().await.map_err(|_| ())?);
            }
            let size = std::str::from_utf8(&line[..line.len() - 2]).map_err(|_| ())?;
            if size.is_empty() || size.len() > 16 || !size.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(());
            }
            let size = u64::from_str_radix(size, 16).map_err(|_| ())?;
            if size > 16 * 1024 * 1024 {
                return Err(());
            }
            if size == 0 {
                let mut end = [0; 2];
                input.read_exact(&mut end).await.map_err(|_| ())?;
                if end != *b"\r\n" {
                    return Err(());
                }
                output.write_all(b"0\r\n\r\n").await.map_err(|_| ())?;
                break;
            }
            output
                .write_all(format!("{size:x}\r\n").as_bytes())
                .await
                .map_err(|_| ())?;
            if tokio::io::copy(&mut input.take(size), output)
                .await
                .map_err(|_| ())?
                != size
            {
                return Err(());
            }
            let mut end = [0; 2];
            input.read_exact(&mut end).await.map_err(|_| ())?;
            if end != *b"\r\n" {
                return Err(());
            }
            output.write_all(b"\r\n").await.map_err(|_| ())?;
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(extra: &str) -> Request {
        super::super::parse(
            format!("GET / HTTP/1.1\r\nHost: w.localhost:1234\r\n{extra}\r\n").as_bytes(),
        )
        .unwrap()
    }
    #[test]
    fn rejects_smuggling_unsupported_cookie_semantics_and_redirect_escapes() {
        for headers in [
            "Content-Length: +1\r\n",
            "Content-Length: 1, 1\r\n",
            "Content-Length: 0\r\nContent-Length: 0\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Connection: cookie\r\n",
            "Upgrade: h2c\r\n",
            "Connection: upgrade\r\n",
            "Upgrade: websocket\r\nConnection: upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: bad\r\n",
        ] {
            assert!(framing(&request(headers)).is_err(), "accepted {headers}");
        }
        for cookie in [
            "sid=abc; Secure",
            "__Host-sid=abc; Path=/",
            "__Secure-sid=abc",
            "__Http-sid=abc",
            "sid=abc; SameSite=None",
            "sid=abc; Partitioned",
            "bad name=abc",
        ] {
            assert!(clean_cookie(cookie).is_err(), "accepted {cookie}");
        }
        assert_eq!(
            clean_cookie("app=ok; Domain=.localhost; Path=/; HttpOnly; SameSite=Lax").unwrap(),
            Some("app=ok; Path=/; HttpOnly; SameSite=Lax".into())
        );
        assert_eq!(
            clean_cookie("_wispnet_session=forged; Domain=.localhost; Path=/").unwrap(),
            None
        );
        for location in [
            "//evil.invalid/",
            "\\evil.invalid/",
            "https://evil.invalid/",
            "javascript:alert(1)",
            "http://evil@w.localhost:1234/",
            " /\\evil.invalid/",
            "\thttps://evil.invalid/",
        ] {
            assert!(
                redirect(location, "http://w.localhost:1234", None).is_err(),
                "accepted {location}"
            );
        }
        assert_eq!(
            redirect("/login?q=1", "http://w.localhost:1234", None).unwrap(),
            "http://w.localhost:1234/login?q=1"
        );
        assert_eq!(
            redirect(
                "http://127.0.0.1:8000/login",
                "http://w.localhost:1234",
                Some("127.0.0.1:8000".parse().unwrap())
            )
            .unwrap(),
            "http://w.localhost:1234/login"
        );
        let r = request("");
        for headers in [
            "Content-Length: 1\r\nContent-Length: 2\r\n",
            "Content-Length: 1\r\nTransfer-Encoding: chunked\r\n",
            "Transfer-Encoding: gzip\r\n",
            "Trailer: Set-Cookie\r\n",
            "Refresh: 0; url=https://evil.invalid/\r\n",
            "Set-Cookie: sid=x; Secure\r\n",
            "Connection: content-length\r\n",
        ] {
            assert!(
                response(
                    format!("HTTP/1.1 200 OK\r\n{headers}\r\n").as_bytes(),
                    &r,
                    "http://w.localhost:1234",
                    None,
                    false
                )
                .is_err(),
                "accepted {headers}"
            );
        }
        let valid=response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive, x-hop\r\nX-Hop: private\r\nCache-Control: public\r\nAccess-Control-Allow-Origin: *\r\nSet-Cookie: app=yes; Domain=.localhost; Path=/\r\n\r\n",&r,"http://w.localhost:1234",None,false).unwrap();
        let output = String::from_utf8(valid.bytes).unwrap();
        assert!(output.contains("Cache-Control: no-store"));
        assert!(!output.contains("x-hop:"));
        assert!(!output.contains("allow-origin"));
        assert!(!output.contains("Domain="));
        assert!(output.contains("Connection: close"));
    }
    #[tokio::test]
    async fn response_framing_preserves_streams_and_never_copies_a_second_message_or_trailers() {
        let mut input = &b"helloHTTP/1.1 200 injected\r\n\r\n"[..];
        let mut output = Vec::new();
        body(&mut input, &mut output, Body::Length(5))
            .await
            .unwrap();
        assert_eq!(output, b"hello");
        let mut input = &b"3\r\none\r\n3\r\ntwo\r\n0\r\n\r\ninjected"[..];
        let mut output = Vec::new();
        body(&mut input, &mut output, Body::Chunked).await.unwrap();
        assert_eq!(output, b"3\r\none\r\n3\r\ntwo\r\n0\r\n\r\n");
        for bad in [
            &b"0\r\nSet-Cookie: injected=yes\r\n\r\n"[..],
            b"1\r\naXX",
            b"+1\r\na\r\n",
            b"1;ext=x\r\na\r\n",
        ] {
            assert!(
                body(&mut &bad[..], &mut Vec::new(), Body::Chunked)
                    .await
                    .is_err()
            );
        }
        assert!(
            body(&mut &b"short"[..], &mut Vec::new(), Body::Length(9))
                .await
                .is_err()
        );
    }
    #[test]
    fn websocket_requires_the_exact_standard_handshake() {
        let r = request(
            "Origin: http://w.localhost:1234\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
        );
        assert!(framing(&r).unwrap().0);
        let valid=b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
        assert!(response(valid, &r, "http://w.localhost:1234", None, true).is_ok());
        assert!(response(valid, &r, "http://w.localhost:1234", None, false).is_err());
        assert!(
            response(
                &String::from_utf8_lossy(valid)
                    .replace("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=", "wrong")
                    .into_bytes(),
                &r,
                "http://w.localhost:1234",
                None,
                true
            )
            .is_err()
        );
    }
}
