//! Optional public-account integration. Private pairing never calls this module.
use crate::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Read,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    sync::Arc,
    time::Duration,
};
use wispnet_agent::http_backend::AuthorizationError;
use wispnet_core::identity::EndpointId;

pub const DEFAULT_URL: &str = "https://cmd.wispnet.app";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    device_code: String,
    user_code: String,
    verification_url: String,
}

// Deliberately no Debug. The owner-only file contains a publishing credential,
// never a backend credential, and none of these fields are returned wholesale.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    version: u8,
    owner: EndpointId,
    pub base_url: String,
    pub publisher_token: Option<String>,
    account: Option<Value>,
    pending: Option<Pending>,
}

pub fn validate_url(value: &str) -> Result<String, String> {
    let url = url::Url::parse(value).map_err(|_| "Invalid command center URL")?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Command center requires an HTTPS origin (HTTP is local-development only)".into(),
        );
    }
    Ok(url.as_str().trim_end_matches('/').into())
}

pub(crate) fn settings(state: &Path, owner: EndpointId) -> Result<Settings, String> {
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(state.join("command-center.json"))
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings {
                version: 1,
                owner,
                base_url: DEFAULT_URL.into(),
                publisher_token: None,
                account: None,
                pending: None,
            });
        }
        Err(_) => return Err("Cannot read command center settings".into()),
    };
    let m = file
        .metadata()
        .map_err(|_| "Cannot inspect command center settings")?;
    if !m.is_file()
        || m.nlink() != 1
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o777 != 0o600
        || m.len() > 16384
    {
        return Err("Unsafe command center settings".into());
    }
    let mut bytes = Vec::new();
    file.take(16385)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read command center settings")?;
    let s: Settings =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid command center settings")?;
    if s.version != 1
        || s.owner != owner
        || validate_url(&s.base_url)? != s.base_url
        || s.publisher_token.as_ref().is_some_and(|t| {
            t.len() < 32 || t.len() > 4096 || t.bytes().any(|b| b <= 32 || b >= 127)
        })
    {
        return Err("Invalid command center settings".into());
    }
    Ok(s)
}

fn save(state: &Path, settings: &Settings) -> Result<(), String> {
    let path = state.join("command-center.json");
    let mut file = wispnet_registry::control::credential_temp(&path)
        .map_err(|_| "Cannot save command center settings")?;
    serde_json::to_writer(&mut file, settings)
        .map_err(|_| "Cannot save command center settings")?;
    file.flush()
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "Cannot save command center settings")?;
    file.persist(path)
        .map_err(|_| "Cannot save command center settings")?;
    std::fs::File::open(state)
        .and_then(|f| f.sync_all())
        .map_err(|_| "Cannot sync command center settings".into())
}

pub(crate) async fn post(
    base: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> Result<Value, String> {
    let response = request(base, path, token, body).await?;
    if !response.status().is_success() {
        return Err("Command center denied the request".into());
    }
    response_json(response).await
}

async fn request(
    base: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> Result<reqwest::Response, String> {
    let base = validate_url(base)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Command center unavailable")?;
    let mut request = client.post(format!("{base}{path}")).json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request
        .send()
        .await
        .map_err(|_| "Command center unavailable".into())
}

async fn response_json(mut response: reqwest::Response) -> Result<Value, String> {
    if response.content_length().is_some_and(|n| n > 32768) {
        return Err("Invalid command center response".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Invalid command center response")?
    {
        if bytes.len() + chunk.len() > 32768 {
            return Err("Invalid command center response".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid command center response".into())
}

pub(crate) fn linked(state: &Path, owner: EndpointId) -> Result<Settings, String> {
    let s = settings(state, owner)?;
    if s.publisher_token.is_none() {
        return Err("Connect a command center account before publishing".into());
    }
    Ok(s)
}

pub(crate) async fn register(
    client: &Client,
    service: &str,
    incarnation: &str,
    name: &str,
    resource: &str,
) -> Result<Settings, String> {
    let s = linked(&client.state, client.endpoint.id())?;
    let result = post(&s.base_url, "/api/command/host/register", s.publisher_token.as_deref(),
        json!({"host":client.endpoint.id(),"service":service,"incarnation":incarnation,"name":name,"resource":resource})).await?;
    if result["ok"] != true || result["resource"] != resource {
        return Err("Command center rejected service registration".into());
    }
    Ok(s)
}

pub(crate) async fn status(client: &Client) -> Result<Value, String> {
    let s = settings(&client.state, client.endpoint.id())?;
    let mut credentials = Vec::new();
    if let Some(authority) = &client.local_authority {
        for service in authority
            .services()
            .await
            .map_err(|_| "Cannot read owned services")?
        {
            if service.kind != wispnet_core::host::ServiceKind::Http {
                continue;
            }
            let credential = wispnet_registry::backend_credentials::status(
                &client.state,
                client.endpoint.id(),
                &service.id,
                &service.incarnation,
                service.backend,
            );
            credentials.push(match credential {
                Ok(status) => {
                    let mode = match status.mode { wispnet_registry::backend_credentials::Mode::PassThrough => "passthrough", wispnet_registry::backend_credentials::Mode::None => "none", wispnet_registry::backend_credentials::Mode::Bearer => "bearer" };
                    json!({"service":service.id,"incarnation":service.incarnation,"mode":mode,"configured":status.configured})
                },
                Err(_) => json!({"service":service.id,"incarnation":service.incarnation,"mode":"invalid","configured":false}),
            });
        }
    }
    Ok(
        json!({"enabled":true,"base_url":s.base_url,"linked":s.publisher_token.is_some(),"account":s.account,
        "pending":s.pending.map(|p|json!({"verification_url":p.verification_url,"user_code":p.user_code})),"credentials":credentials}),
    )
}

pub(crate) async fn configure_backend(client: Arc<Client>, value: &Value) -> Result<Value, String> {
    use wispnet_registry::backend_credentials::{self, Mode};
    let authority = client
        .local_authority
        .as_ref()
        .ok_or("Backend credentials require local ownership")?;
    let id = crate::argument(value, "service")?;
    let incarnation = crate::argument(value, "incarnation")?;
    let binding = authority
        .services()
        .await
        .map_err(|_| "Cannot read owned services")?
        .into_iter()
        .find(|s| {
            s.id == id
                && s.incarnation == incarnation
                && s.kind == wispnet_core::host::ServiceKind::Http
        })
        .ok_or("Refresh the owned HTTP service before configuring authentication")?;
    let mode = match crate::argument(value, "mode")? {
        "passthrough" | "pass_through" => Mode::PassThrough,
        "none" => Mode::None,
        "bearer" => Mode::Bearer,
        _ => return Err("Invalid backend authentication mode".into()),
    };
    if matches!(mode, Mode::Bearer) && client.publications.has_unmanaged(&client, id)? {
        return Err("Disable legacy public access before storing a backend credential".into());
    }
    let result = backend_credentials::set(
        &client.state,
        client.endpoint.id(),
        id,
        incarnation,
        binding.backend,
        mode,
        value["token"].as_str(),
    )?;
    client
        .gateway
        .credentials_changed(client.endpoint.id(), id)
        .await;
    Ok(
        json!({"service":id,"incarnation":incarnation,"mode":result.mode,"configured":result.configured}),
    )
}

pub(crate) async fn command(client: Arc<Client>, value: &Value) -> Result<Value, String> {
    let authority = client
        .local_authority
        .as_ref()
        .ok_or("Command center administration requires local ownership")?;
    // Account commands read, await the hub, then replace the same settings file.
    // Keep the whole operation ordered across desktop and CLI callers.
    let _account = client.command_center_lock.lock().await;
    let host = client.endpoint.id();
    match crate::argument(value, "op")? {
        "command_center_status" => status(&client).await,
        "command_center_link" => {
            let mut s = settings(&client.state, host)?;
            let base = validate_url(value["base_url"].as_str().unwrap_or(&s.base_url))?;
            if s.publisher_token.is_some() {
                return Err("Unlink the current public account before linking another".into());
            }
            let name = authority.device_id();
            let nonce = crate::token();
            let proof = format!("wispnet-command-link-v1\n{base}\n{host}\n{name}\n{nonce}");
            let signature: String = client
                .key
                .sign(proof.as_bytes())
                .to_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let response = post(
                &base,
                "/api/command/links/start",
                None,
                json!({"host":host,"name":name,"nonce":nonce,"signature":signature}),
            )
            .await?;
            let verification = response["verification_url"]
                .as_str()
                .ok_or("Invalid link response")?;
            let verification_url =
                url::Url::parse(verification).map_err(|_| "Invalid link response")?;
            let origin = url::Url::parse(&base).map_err(|_| "Invalid link response")?;
            if verification_url.origin() != origin.origin() {
                return Err("Invalid link destination".into());
            }
            let pending = Pending {
                device_code: response["device_code"]
                    .as_str()
                    .filter(|s| s.len() >= 32 && s.len() <= 4096)
                    .ok_or("Invalid link response")?
                    .into(),
                user_code: response["user_code"]
                    .as_str()
                    .filter(|s| s.len() <= 64)
                    .ok_or("Invalid link response")?
                    .into(),
                verification_url: verification.into(),
            };
            let result = json!({"state":"pending","verification_url":pending.verification_url,"user_code":pending.user_code});
            s.base_url = base;
            s.pending = Some(pending);
            save(&client.state, &s)?;
            Ok(result)
        }
        "command_center_poll" => {
            let mut s = settings(&client.state, host)?;
            let pending = s.pending.as_ref().ok_or("Start account linking first")?;
            for (field, actual) in [
                ("expected_user_code", pending.user_code.as_str()),
                (
                    "expected_verification_url",
                    pending.verification_url.as_str(),
                ),
            ] {
                if value
                    .get(field)
                    .is_some_and(|expected| expected.as_str() != Some(actual))
                {
                    return Err("Account sign-in request changed; restart sign-in".into());
                }
            }
            let response = post(
                &s.base_url,
                "/api/command/links/poll",
                None,
                json!({"device_code":pending.device_code}),
            )
            .await?;
            if response["state"] == "linked" {
                let token = response["publisher_token"]
                    .as_str()
                    .filter(|t| {
                        t.len() >= 32 && t.len() <= 4096 && t.bytes().all(|b| b > 32 && b < 127)
                    })
                    .ok_or("Invalid link response")?;
                s.publisher_token = Some(token.into());
                s.account = Some(response["account"].clone());
                s.pending = None;
                save(&client.state, &s)?;
                Ok(json!({"state":"linked","account":s.account}))
            } else {
                Ok(json!({"state":"pending"}))
            }
        }
        "command_center_unlink" => {
            let mut s = settings(&client.state, host)?;
            // Local ownership always permits disconnection, including after a
            // dashboard unlink or while the hub is unavailable. Stop routes and
            // forget the credential before attempting remote revocation.
            client.publications.disable_managed(client.clone()).await?;
            let publisher = s.publisher_token.take();
            s.account = None;
            s.pending = None;
            save(&client.state, &s)?;
            let revoked = if let Some(token) = publisher {
                Some(
                    matches!(post(&s.base_url, "/api/command/host/unlink", Some(&token), json!({"host":host})).await,
                    Ok(response) if response["ok"] == true),
                )
            } else {
                None
            };
            let mut result = status(&client).await?;
            result["remote_revocation"] = json!(match revoked {
                Some(true) => "confirmed",
                Some(false) => "unconfirmed",
                None => "not_requested",
            });
            if revoked == Some(false) {
                result["warning"] = json!(
                    "Publications stopped on this computer. Remote revocation could not be confirmed; check Command Center to disconnect this computer."
                );
            }
            Ok(result)
        }
        _ => Err("Unsupported command center operation".into()),
    }
}

/// Authority is the original service-audience token, not a forwarded user header.
pub(crate) struct PublicAuthorization {
    pub base_url: String,
    pub publisher_token: String,
    pub host: EndpointId,
    pub service: String,
    pub incarnation: String,
    pub resource: String,
}
impl wispnet_agent::http_backend::RequestAuthorization for PublicAuthorization {
    fn authorize(
        &self,
        headers: &hyper::HeaderMap,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), AuthorizationError>> + Send + '_>,
    > {
        let headers = headers.clone();
        Box::pin(async move {
            if headers.get_all(hyper::header::AUTHORIZATION).iter().count() != 1 {
                return Err(AuthorizationError::Denied);
            }
            let token = headers
                .get(hyper::header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.strip_prefix("Bearer "))
                .filter(|t| !t.is_empty() && t.len() <= 8192)
                .ok_or(AuthorizationError::Denied)?;
            let response = request(&self.base_url, "/api/command/authorize", Some(&self.publisher_token),
                json!({"host":self.host,"service":self.service,"incarnation":self.incarnation,"resource":self.resource,"token":token}))
                .await.map_err(|_| AuthorizationError::Unavailable)?;
            let status = response.status();
            let result = response_json(response)
                .await
                .map_err(|_| AuthorizationError::Unavailable)?;
            if status.is_success() && result["allowed"] == true {
                return Ok(());
            }
            // Only an explicit service-token denial means the visitor needs new
            // credentials. Publisher/configuration failures and outages cannot
            // be repaired by making the visitor reconnect their OAuth account.
            if status == reqwest::StatusCode::FORBIDDEN
                && result["error"] == "Service access denied"
            {
                return Err(AuthorizationError::Denied);
            }
            Err(AuthorizationError::Unavailable)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::oneshot,
    };

    async fn account_client(base_url: &str) -> (tempfile::TempDir, Arc<Client>) {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let key = wispnet_core::identity::SecretKey::generate();
        let owner = key.public();
        wispnet_registry::host::initialize(state.path(), owner).unwrap();
        let config = serde_json::from_value(json!({"bind_addr":"127.0.0.1:0"})).unwrap();
        let endpoint = wispnet_agent::tunnel::bind_configured_client(&key, Some(&config))
            .await
            .unwrap();
        let client = Client::from_endpoint(
            endpoint,
            key,
            state.path().into(),
            config,
            wispnet_registry::host::Authority::open(state.path(), owner).unwrap(),
        );
        let mut s = settings(state.path(), owner).unwrap();
        s.base_url = base_url.into();
        s.pending = Some(Pending {
            device_code: "device-fixture-0123456789abcdefghijk".into(),
            user_code: "ABCD-1234-5678".into(),
            verification_url: format!("{base_url}/connect?code=ABCD-1234-5678"),
        });
        save(state.path(), &s).unwrap();
        (state, client)
    }

    async fn hub_request(listener: &TcpListener) -> (TcpStream, String, Value) {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                assert!(headers.len() < 16384);
                headers.push(stream.read_u8().await.unwrap());
            }
            let headers = String::from_utf8(headers).unwrap();
            let length: usize = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .unwrap()
                .1
                .trim()
                .parse()
                .unwrap();
            assert!(length <= 16384);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            (stream, headers, serde_json::from_slice(&body).unwrap())
        })
        .await
        .unwrap()
    }

    async fn hub_reply(mut stream: TcpStream, response: Value) {
        let body = serde_json::to_vec(&response).unwrap();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
    }

    #[tokio::test]
    async fn public_authorization_distinguishes_service_denial_from_upstream_failures() {
        use wispnet_agent::http_backend::RequestAuthorization;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let authorization = PublicAuthorization {
            base_url,
            publisher_token: "publisher-fixture".into(),
            host: wispnet_core::identity::SecretKey::generate().public(),
            service: "mcp".into(),
            incarnation: "a".repeat(64),
            resource: "https://service.example/mcp".into(),
        };
        let mut headers = hyper::HeaderMap::new();
        headers.insert(
            "authorization",
            "Bearer original-fixture-token".parse().unwrap(),
        );
        let cases = [
            (
                503,
                r#"{"error":"Service access denied"}"#,
                Err(AuthorizationError::Unavailable),
            ),
            (
                429,
                r#"{"error":"Try again later"}"#,
                Err(AuthorizationError::Unavailable),
            ),
            (
                400,
                r#"{"error":"Invalid request"}"#,
                Err(AuthorizationError::Unavailable),
            ),
            (
                401,
                r#"{"error":"Upstream authentication required"}"#,
                Err(AuthorizationError::Unavailable),
            ),
            (
                403,
                r#"{"error":"Publisher not authorized"}"#,
                Err(AuthorizationError::Unavailable),
            ),
            (
                403,
                "upstream unavailable",
                Err(AuthorizationError::Unavailable),
            ),
            (
                200,
                r#"{"allowed":false}"#,
                Err(AuthorizationError::Unavailable),
            ),
            (200, "invalid json", Err(AuthorizationError::Unavailable)),
            (
                403,
                r#"{"error":"Service access denied"}"#,
                Err(AuthorizationError::Denied),
            ),
            (200, r#"{"allowed":true}"#, Ok(())),
        ];
        for (status, body, expected) in cases {
            let check = authorization.authorize(&headers);
            let reply = async {
                let (mut stream, request, value) = hub_request(&listener).await;
                assert!(request.starts_with("POST /api/command/authorize "));
                assert_eq!(value["token"], "original-fixture-token");
                stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            };
            let (actual, ()) = tokio::join!(check, reply);
            assert_eq!(actual, expected, "status {status}, body {body}");
        }
        drop(listener);
        assert_eq!(
            authorization.authorize(&headers).await,
            Err(AuthorizationError::Unavailable)
        );
    }

    #[tokio::test]
    async fn poll_rejects_replaced_or_malformed_expectations_before_contacting_hub() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (_state, client) = account_client(&base).await;
        for (field, expected) in [
            ("expected_user_code", json!("OLD-CODE")),
            ("expected_user_code", Value::Null),
            ("expected_user_code", json!(true)),
            ("expected_verification_url", json!("https://other.example/")),
            ("expected_verification_url", Value::Null),
            ("expected_verification_url", json!({"url":"changed"})),
        ] {
            let mut request = json!({"op":"command_center_poll"});
            request[field] = expected;
            let error = client.command(&request).await.unwrap_err();
            assert!(error.contains("sign-in request changed"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err()
        );
        let hub = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, headers, body) = hub_request(&listener).await;
                assert!(headers.starts_with("POST /api/command/links/poll "));
                assert_eq!(body["device_code"], "device-fixture-0123456789abcdefghijk");
                hub_reply(stream, json!({"state":"pending"})).await;
            }
        });
        // Matching expectations and older callers without them both still work.
        for request in [
            json!({"op":"command_center_poll","expected_user_code":"ABCD-1234-5678",
                "expected_verification_url":format!("{base}/connect?code=ABCD-1234-5678")}),
            json!({"op":"command_center_poll"}),
        ] {
            assert_eq!(client.command(&request).await.unwrap()["state"], "pending");
        }
        hub.await.unwrap();
        client.endpoint.close().await;
    }

    #[tokio::test]
    async fn unlink_waits_for_inflight_poll_and_cannot_be_overwritten_by_its_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (state, client) = account_client(&base).await;
        let (started, received) = oneshot::channel();
        let (release, waiting) = oneshot::channel();
        let hub = tokio::spawn(async move {
            let (stream, headers, _) = hub_request(&listener).await;
            assert!(headers.starts_with("POST /api/command/links/poll "));
            started.send(()).unwrap();
            waiting.await.unwrap();
            hub_reply(
                stream,
                json!({"state":"linked",
                "publisher_token":"wph_fixture_0123456789abcdefghijklmnopqrstuvwxyz",
                "account":{"id":"usr_fixture","name":"Fixture"}}),
            )
            .await;
            let (stream, headers, _) = hub_request(&listener).await;
            assert!(headers.starts_with("POST /api/command/host/unlink "));
            assert!(headers.contains("Bearer wph_fixture_0123456789abcdefghijklmnopqrstuvwxyz"));
            hub_reply(stream, json!({"ok":true})).await;
        });
        let polling = client.clone();
        let poll =
            tokio::spawn(
                async move { polling.command(&json!({"op":"command_center_poll"})).await },
            );
        received.await.unwrap();
        let unlinking = client.clone();
        let mut unlink = tokio::spawn(async move {
            unlinking
                .command(&json!({"op":"command_center_unlink"}))
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut unlink)
                .await
                .is_err()
        );
        assert!(
            settings(state.path(), client.endpoint.id())
                .unwrap()
                .pending
                .is_some()
        );
        release.send(()).unwrap();
        assert_eq!(poll.await.unwrap().unwrap()["state"], "linked");
        let result = unlink.await.unwrap().unwrap();
        assert_eq!(result["linked"], false);
        assert_eq!(result["remote_revocation"], "confirmed");
        let final_settings = settings(state.path(), client.endpoint.id()).unwrap();
        assert!(final_settings.publisher_token.is_none());
        assert!(final_settings.account.is_none());
        assert!(final_settings.pending.is_none());
        hub.await.unwrap();
        client.endpoint.close().await;
    }

    #[test]
    fn protected_identity_bound_settings_and_redacted_origin() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let host = wispnet_core::identity::SecretKey::generate().public();
        let mut s = settings(temp.path(), host).unwrap();
        s.publisher_token = Some("publishing-fixture-secret-not-for-logs".into());
        save(temp.path(), &s).unwrap();
        assert!(
            settings(temp.path(), host)
                .unwrap()
                .publisher_token
                .is_some()
        );
        assert!(
            settings(
                temp.path(),
                wispnet_core::identity::SecretKey::generate().public()
            )
            .is_err()
        );
        std::fs::set_permissions(
            temp.path().join("command-center.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(settings(temp.path(), host).is_err());
        for bad in [
            "http://example.com",
            "https://a.example/path",
            "https://user:secret@a.example",
            "https://a.example/?token=x",
        ] {
            assert!(validate_url(bad).is_err());
        }
    }
}
