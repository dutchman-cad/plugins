// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::{
    bindings::panel::solidcraft::plugin::{net, types::PluginError as E},
    manifest::{MIB, Manifest},
};
use std::{
    collections::BTreeMap,
    net::IpAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Settings {
    pub addresses: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, String>,
    pub selection: Option<PathBuf>,
    pub confirm_physical: bool,
}

pub fn allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(
            || {
                ip.is_loopback()
                    || (ip.segments()[0] & 0xfe00 == 0xfc00)
                    || (ip.segments()[0] & 0xffc0 == 0xfe80)
            },
            |v| allowed(IpAddr::V4(v)),
        ),
    }
}
fn invalid(s: impl ToString) -> E {
    E::Invalid(s.to_string())
}
fn failed(_: impl std::fmt::Display) -> E {
    E::Failed("Local service request failed".into())
}
fn denied(s: &str) -> E {
    E::Denied(s.into())
}

pub fn origin(base: &str) -> Result<reqwest::Url, E> {
    let url = reqwest::Url::parse(base).map_err(|_| invalid("Invalid local address"))?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid("Local address must be an HTTP origin"));
    }
    match url.host() {
        Some(url::Host::Ipv4(ip)) if allowed(ip.into()) => {}
        Some(url::Host::Ipv6(ip)) if allowed(ip.into()) => {}
        Some(url::Host::Domain(name)) if name.ends_with(".local") => {}
        _ => return Err(denied("Local address is outside allowed ranges")),
    }
    Ok(url)
}

fn checked(addresses: &[std::net::SocketAddr]) -> bool {
    !addresses.is_empty() && addresses.iter().all(|a| allowed(a.ip()))
}

pub struct Confirmation {
    token: String,
    expires: Instant,
}
impl Confirmation {
    pub fn new() -> anyhow::Result<Self> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes)
            .map_err(|_| anyhow::anyhow!("Confirmation randomness unavailable"))?;
        Ok(Self {
            token: bytes.iter().map(|b| format!("{b:02x}")).collect(),
            expires: Instant::now() + Duration::from_secs(60),
        })
    }
}

#[derive(Default)]
pub struct FetchBudget {
    requests: usize,
    mesh: bool,
}
impl FetchBudget {
    pub async fn fetch(
        &mut self,
        manifest: &Manifest,
        settings: &Settings,
        confirmation: Option<&Confirmation>,
        req: net::Request,
    ) -> Result<net::Response, E> {
        self.requests += 1;
        if self.requests > 64 {
            return Err(E::Limit("At most 64 requests per call".into()));
        }
        if req
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("Plugin-Confirmation"))
        {
            return Err(denied("Plugin-Confirmation is host-only"));
        }
        let rest = req
            .url
            .strip_prefix("local:")
            .ok_or_else(|| denied("Only approved local services are available"))?;
        let (name, path) = rest
            .split_once('/')
            .ok_or_else(|| invalid("Local URL requires a path"))?;
        let service = manifest
            .permissions
            .local
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| denied("Unknown local service"))?;
        if path.starts_with('/')
            || path.contains(['#', '\\'])
            || req.url.chars().any(char::is_control)
        {
            return Err(invalid("Invalid local path"));
        }
        let base = settings
            .addresses
            .get(name)
            .ok_or_else(|| E::Failed(format!("Set the address of {name} in host settings")))?;
        let mut url = origin(base)?;
        // Parse the complete URL, then compare the authority rather than letting join replace it.
        let target = reqwest::Url::parse(&format!("{}/{path}", url.as_str().trim_end_matches('/')))
            .map_err(|_| invalid("Invalid local path"))?;
        if target.origin() != url.origin() {
            return Err(denied("Local authority cannot change"));
        }
        url = target;
        if req.headers.len() > 64 || req.headers.iter().any(|(k, v)| k.len() + v.len() > 8192) {
            return Err(E::Limit("Request headers exceed limit".into()));
        }
        let mesh = req.mesh.is_some();
        let body = if let Some(mesh) = req.mesh {
            if !service.mesh {
                return Err(denied("Mesh permission required"));
            }
            if !req.body.is_empty() {
                return Err(invalid("Mesh requires an empty body"));
            }
            if self.mesh {
                return Err(E::Limit("One mesh request per call".into()));
            }
            self.mesh = true;
            if !mesh.bodies.is_empty() {
                return Err(invalid("Named bodies unavailable with file selection"));
            }
            if !matches!(mesh.quality, net::MeshQuality::Standard) {
                return Err(invalid("File selection supports standard quality only"));
            }
            let path = settings
                .selection
                .as_ref()
                .ok_or_else(|| E::Failed("Set --selection-3mf in the host".into()))?;
            use std::io::Read;
            if !std::fs::metadata(path).map_err(failed)?.is_file() {
                return Err(E::Failed("Selection must be a regular 3MF file".into()));
            }
            let file = std::fs::File::open(path).map_err(failed)?;
            let size = file.metadata().map_err(failed)?.len();
            if size > (256 * MIB) as u64 {
                return Err(E::Failed(format!("Mesh size {size} exceeds 256 MiB")));
            }
            let mut bytes = Vec::new();
            file.take((256 * MIB + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(failed)?;
            if bytes.len() > 256 * MIB {
                return Err(E::Failed(format!(
                    "Mesh size {} exceeds 256 MiB",
                    bytes.len()
                )));
            }
            bytes
        } else {
            req.body
        };
        if !mesh && body.len() > MIB {
            return Err(E::Limit("Request body exceeds 1 MiB".into()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| invalid("Missing local host"))?
            .trim_matches(['[', ']']);
        // A new client per request has no cached DNS, pooled connection, proxy, cookie or redirect.
        let addresses: Vec<_> = tokio::net::lookup_host((
            host,
            url.port_or_known_default()
                .ok_or_else(|| invalid("Missing port"))?,
        ))
        .await
        .map_err(failed)?
        .collect();
        if !checked(&addresses) {
            return Err(denied("Resolved local address is outside allowed ranges"));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(failed)?;
        let method = match req.method {
            net::Method::Get => reqwest::Method::GET,
            net::Method::Post => reqwest::Method::POST,
            net::Method::Put => reqwest::Method::PUT,
            net::Method::Patch => reqwest::Method::PATCH,
            net::Method::Delete => reqwest::Method::DELETE,
        };
        let mut request = client.request(method, url.clone());
        for (key, value) in req.headers {
            if [
                "host",
                "content-length",
                "transfer-encoding",
                "connection",
                "user-agent",
            ]
            .contains(&key.to_ascii_lowercase().as_str())
            {
                return Err(denied("Reserved HTTP header"));
            }
            if (mesh && key.eq_ignore_ascii_case("content-type"))
                || service
                    .secret
                    .as_ref()
                    .is_some_and(|s| s.header.eq_ignore_ascii_case(&key))
            {
                continue;
            }
            request = request.header(&key, &value);
        }
        let secret = service
            .secret
            .as_ref()
            .and_then(|s| settings.secrets.get(&s.name));
        if let (Some(s), Some(value)) = (&service.secret, secret) {
            request = request.header(&s.header, format!("{}{value}", s.prefix));
        }
        let token = confirmation
            .filter(|c| Instant::now() < c.expires)
            .map(|c| c.token.as_str());
        if let Some(token) = token {
            request = request.header("Plugin-Confirmation", token);
        }
        if mesh {
            request = request.header("Content-Type", "model/3mf");
        }
        request = request.header(
            "User-Agent",
            format!(
                "SolidCraft/reference plugin/{}/{}",
                manifest.id, manifest.version
            ),
        );
        let mut response = request.body(body).send().await.map_err(failed)?;
        let status = response.status().as_u16();
        let redact = |s: &str| {
            let mut s = s.to_string();
            for value in [secret.map(String::as_str), token]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
            {
                s = s.replace(value, "[secret]");
            }
            s
        };
        let mut headers = Vec::new();
        for (k, v) in response.headers() {
            let header_bytes = k.as_str().len() + v.len();
            if header_bytes > 8192 || headers.len() >= 64 {
                return Err(E::Limit("Response headers exceed limit".into()));
            }
            headers.push((redact(k.as_str()), redact(v.to_str().map_err(invalid)?)));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(failed)? {
            if bytes.len() + chunk.len() > 8 * MIB {
                return Err(E::Limit("Response body exceeds limit".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        // Redact exact byte sequences without altering non-text response data.
        for value in [secret.map(String::as_str), token]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
        {
            let pattern = value.as_bytes();
            let mut redacted = Vec::new();
            let mut tail = bytes.as_slice();
            while let Some(at) = tail.windows(pattern.len()).position(|w| w == pattern) {
                if redacted.len().saturating_add(at).saturating_add(8) > 8 * MIB {
                    return Err(E::Limit("Redacted response exceeds limit".into()));
                }
                redacted.extend_from_slice(&tail[..at]);
                redacted.extend_from_slice(b"[secret]");
                tail = &tail[at + pattern.len()..];
            }
            if redacted.len().saturating_add(tail.len()) > 8 * MIB {
                return Err(E::Limit("Redacted response exceeds limit".into()));
            }
            redacted.extend_from_slice(tail);
            bytes = redacted;
        }
        Ok(net::Response {
            status,
            headers,
            body: bytes,
        })
    }
}

/// One coalesced timer, owned by the visible text surface rather than a guest Store.
#[derive(Default)]
pub struct Refresh {
    interval: Option<Duration>,
    due: Option<Instant>,
    visible: bool,
}
impl Refresh {
    pub fn schedule(&mut self, seconds: u32, now: Instant) {
        self.interval = Some(Duration::from_secs(u64::from(seconds.max(2))));
        self.due = self.interval.filter(|_| self.visible).map(|d| now + d);
    }
    pub fn visible(&mut self, visible: bool, now: Instant) {
        if self.visible != visible {
            self.visible = visible;
            self.due = self.interval.filter(|_| visible).map(|d| now + d);
        }
    }
    pub fn due(&self) -> Option<Instant> {
        self.due
    }
    pub fn take(&mut self, now: Instant) -> bool {
        if self.due.is_some_and(|d| d <= now) {
            self.due = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn t29_local_ranges_and_origins() {
        for ip in [
            "127.0.0.1",
            "127.255.1.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "::1",
            "fc00::1",
            "fdff::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(allowed(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "8.8.8.8",
            "172.32.0.1",
            "100.64.0.1",
            "0.0.0.0",
            "::",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
            "fec0::1",
        ] {
            assert!(!allowed(ip.parse().unwrap()), "{ip}");
        }
        for base in [
            "http://8.8.8.8",
            "http://[::ffff:8.8.8.8]",
            "http://localhost",
            "http://public.example",
            "http://127.0.0.1/path",
            "http://u@127.0.0.1",
            "http://127.0.0.1?q=1",
            "file:///file",
        ] {
            assert!(origin(base).is_err(), "{base}");
        }
        assert!(origin("http://printer.local:8730").is_ok());
    }
    #[test]
    fn t34_refresh_floor_replace_coalesce_visibility() {
        let mut refresh = Refresh::default();
        let now = Instant::now();
        refresh.visible(true, now);
        refresh.schedule(0, now);
        assert!(!refresh.take(now + Duration::from_secs(1)));
        assert!(refresh.take(now + Duration::from_secs(2)));
        assert!(!refresh.take(now + Duration::from_secs(20)));
        refresh.schedule(20, now);
        refresh.schedule(3, now);
        assert!(refresh.take(now + Duration::from_secs(3)));
        refresh.schedule(2, now);
        refresh.visible(false, now);
        assert!(!refresh.take(now + Duration::from_secs(30)));
        refresh.visible(true, now + Duration::from_secs(30));
        assert!(!refresh.take(now + Duration::from_secs(31)));
        assert!(refresh.take(now + Duration::from_secs(32)));
    }
    fn manifest() -> Manifest {
        serde_json::from_str(include_str!("../../examples/print/plugin.json")).unwrap()
    }
    fn request() -> net::Request {
        net::Request {
            method: net::Method::Get,
            url: "local:dutchman-print/echo".into(),
            headers: vec![],
            body: vec![],
            mesh: None,
        }
    }
    fn fake(
        reply: impl FnOnce(&str) -> String + Send + 'static,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
            }
            let request = String::from_utf8(bytes).unwrap();
            let length = request
                .lines()
                .filter_map(|s| s.split_once(':'))
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.trim().parse::<u64>().ok())
                .unwrap_or(0);
            std::io::copy(&mut (&mut socket).take(length), &mut std::io::sink()).unwrap();
            socket.write_all(reply(&request).as_bytes()).unwrap();
            request
        });
        (address, task)
    }
    #[tokio::test]
    async fn t28_t31_secrets_confirmation_redaction_and_expiry() {
        let mut manifest = manifest();
        manifest.permissions.local[0].secret = Some(crate::manifest::Secret {
            name: "password".into(),
            label: "Password".into(),
            header: "Authorization".into(),
            prefix: "Bearer ".into(),
        });
        let confirmation = Confirmation::new().unwrap();
        let echo = confirmation.token.clone();
        assert_eq!(echo.len(), 64);
        assert!(
            echo.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        let (address, server) = fake(move |_| {
            let body = format!("password-value {echo}");
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nX-Echo: {echo}\r\n{echo}: password-value\r\n\r\n{body}",
                body.len()
            )
        });
        let mut settings = Settings::default();
        settings.addresses.insert("dutchman-print".into(), address);
        settings
            .secrets
            .insert("password".into(), "password-value".into());
        let mut req = request();
        req.headers
            .push(("Authorization".into(), "plugin-value".into()));
        let response = FetchBudget::default()
            .fetch(&manifest, &settings, Some(&confirmation), req)
            .await
            .unwrap();
        assert_eq!(response.body, b"[secret] [secret]");
        assert!(
            response
                .headers
                .iter()
                .all(|(k, v)| !k.contains(&confirmation.token) && !v.contains(&confirmation.token))
        );
        let sent = server.join().unwrap().to_lowercase();
        assert!(sent.contains("authorization: bearer password-value"));
        assert!(!sent.contains("plugin-value"));
        assert!(sent.contains(&format!("plugin-confirmation: {}", confirmation.token)));
        let (address, server) = fake(|_| {
            "HTTP/1.1 302 Found\r\nLocation: http://8.8.8.8/never\r\nContent-Length: 0\r\n\r\n"
                .into()
        });
        settings.addresses.insert("dutchman-print".into(), address);
        let expired = Confirmation {
            token: confirmation.token.clone(),
            expires: Instant::now() - Duration::from_secs(1),
        };
        let response = FetchBudget::default()
            .fetch(&manifest, &settings, Some(&expired), request())
            .await
            .unwrap();
        assert_eq!(response.status, 302);
        assert!(
            !server
                .join()
                .unwrap()
                .to_lowercase()
                .contains("plugin-confirmation")
        );
    }
    #[test]
    fn t29_all_dns_answers_must_be_local() {
        let private = "127.0.0.1:8730".parse().unwrap();
        let public = "8.8.8.8:8730".parse().unwrap();
        assert!(checked(&[private]));
        assert!(!checked(&[private, public]));
        assert!(!checked(&[public]));
        assert!(!checked(&[]));
    }
    #[tokio::test]
    async fn t37_oversized_mesh_sends_nothing() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len((256 * MIB + 1) as u64).unwrap();
        let mut settings = Settings {
            selection: Some(file.path().into()),
            ..Default::default()
        };
        settings
            .addresses
            .insert("dutchman-print".into(), "http://127.0.0.1:1".into());
        let mut req = request();
        req.mesh = Some(net::MeshBody {
            bodies: vec![],
            format: net::MeshFormat::ThreeMf,
            quality: net::MeshQuality::Standard,
        });
        assert!(
            matches!(FetchBudget::default().fetch(&manifest(), &settings, None, req).await, Err(E::Failed(s)) if s.contains("268435457"))
        );
    }

    #[tokio::test]
    async fn t37_exact_mesh_cap_and_request_limits() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len((256 * MIB) as u64).unwrap();
        let (address, server) = fake(|_| "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".into());
        let mut settings = Settings {
            selection: Some(file.path().into()),
            ..Default::default()
        };
        settings.addresses.insert("dutchman-print".into(), address);
        let mut req = request();
        req.method = net::Method::Post;
        req.mesh = Some(net::MeshBody {
            bodies: vec![],
            format: net::MeshFormat::ThreeMf,
            quality: net::MeshQuality::Standard,
        });
        let response = FetchBudget::default()
            .fetch(&manifest(), &settings, None, req)
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        let sent = server.join().unwrap().to_ascii_lowercase();
        assert!(sent.contains("content-type: model/3mf"));
        assert!(sent.contains("content-length: 268435456"));
        let mut budget = FetchBudget {
            requests: 64,
            mesh: false,
        };
        assert!(matches!(
            budget.fetch(&manifest(), &settings, None, request()).await,
            Err(E::Limit(_))
        ));
        let mut req = request();
        req.body = vec![0; MIB + 1];
        assert!(matches!(
            FetchBudget::default()
                .fetch(&manifest(), &settings, None, req)
                .await,
            Err(E::Limit(_))
        ));
    }
}
