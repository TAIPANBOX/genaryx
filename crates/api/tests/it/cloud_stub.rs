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
use std::sync::Arc;
use std::time::Duration;

/// path -> (status line, body); anything else is a 404.
type Routes = HashMap<&'static str, (&'static str, String)>;

/// Serve `routes` on a fresh loopback port and return its base URL.
pub fn spawn_cloud(routes: Vec<(&'static str, &'static str, String)>) -> String {
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
            serve(stream, &routes);
        }
    });
    base
}

fn serve(stream: TcpStream, routes: &Routes) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let path = target.split('?').next().unwrap_or("").to_string();
    let mut content_length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() {
            return;
        }
        let t = h.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            break;
        }
        if let Some((n, v)) = t.split_once(':')
            && n.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok();
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
    let state = MoneyState::pending();
    *state.inner.lock().await = MoneyInner::Ready(MoneyClient {
        client: Arc::new(CloudClient::new(base, "test-token").expect("client")),
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
