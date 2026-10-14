//! A hand-rolled stub TokenFuse Cloud, shared by the Money tests that came
//! after `money_runs_budgets_test.rs` (which keeps its own copy, unchanged).
//!
//! It routes by path only: each route is a status line and a body, anything
//! else answers 404 the way an older Cloud without the route does. Listeners
//! bind port 0, so files in this one test binary never collide.

use genaryx_api::money::env::EnvSource;
use genaryx_api::money::state::{MoneyClient, MoneyInner, MoneyState};
use genaryx_connectors::CloudClient;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// path -> (status line, body); anything else is a 404.
type Routes = HashMap<&'static str, (&'static str, String)>;

/// One request the stub received.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Serve `routes` on a fresh loopback port and return its base URL.
pub fn spawn_cloud(routes: Vec<(&'static str, &'static str, String)>) -> String {
    spawn_recording_cloud(routes).0
}

/// [`spawn_cloud`], keeping every request it received.
pub fn spawn_recording_cloud(
    routes: Vec<(&'static str, &'static str, String)>,
) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let log = Arc::clone(&seen);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let base = format!("http://{}", listener.local_addr().expect("stub address"));
    let routes: Arc<Routes> = Arc::new(
        routes
            .into_iter()
            .map(|(path, status, body)| (path, (status, body)))
            .collect(),
    );
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &routes, &log);
        }
    });
    (base, seen)
}

fn serve(stream: TcpStream, routes: &Routes, log: &Mutex<Vec<Seen>>) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let method = line.split_whitespace().next().unwrap_or("").to_string();
    let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let path = target.split('?').next().unwrap_or("").to_string();
    let mut content_length = 0usize;
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() {
            return;
        }
        let t = h.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            break;
        }
        if let Some((n, v)) = t.split_once(':') {
            if n.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
            headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok();
    }
    if let Ok(mut seen) = log.lock() {
        seen.push(Seen {
            method,
            path: path.clone(),
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        });
    }
    let (status, out) = routes
        .get(path.as_str())
        .cloned()
        .unwrap_or(("404 Not Found", r#"{"error":"not found"}"#.to_string()));
    let mut w = reader.into_inner();
    let _ = write!(
        w,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    );
    let _ = w.flush();
}

/// A Ready Money panel pointed at `base`. No device is attached: reads only.
pub async fn money_at(base: &str) -> MoneyState {
    ready(CloudClient::new(base, "test-token").expect("client"), base).await
}

/// A Ready Money panel with a paired software device attached, so the signed
/// mutations reach the stub.
pub async fn money_with_device_at(base: &str) -> MoneyState {
    let mut client = CloudClient::new(base, "test-token").expect("client");
    let signer = genaryx_signing::SoftwareSigner::generate().expect("software signer");
    client.attach_device("dev-test", "device-token", Box::new(signer));
    ready(client, base).await
}

async fn ready(client: CloudClient, base: &str) -> MoneyState {
    let state = MoneyState::pending();
    *state.inner.lock().await = MoneyInner::Ready(MoneyClient {
        client: Arc::new(client),
        source: EnvSource::EnvFallback,
        cloud_url: base.to_string(),
        org_domain: "local".to_string(),
        operator: "user://local/test".to_string(),
        host: "test".to_string(),
        sig_fpr: "software-signed",
        bus: None,
    });
    state
}
