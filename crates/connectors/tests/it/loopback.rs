//! A loopback HTTP stub for this crate's client tests.
//!
//! It routes by `"METHOD /path"` (or a bare `"/path"`, any method), answers
//! each route with a fixed status line and body, and keeps every request it
//! received. Anything unrouted answers 404, the way a plane without the route
//! does. Listeners bind port 0, so files in this one test binary never
//! collide.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One request the stub received.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    /// The request target as sent, query included and still percent-encoded.
    pub target: String,
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

/// A running stub: its base URL and what it has been sent so far.
pub struct Stub {
    pub base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Stub {
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().expect("request log").clone()
    }
}

type Routes = HashMap<String, (String, String)>;

/// Serve `routes`, each `(key, status line, body)`, on a fresh loopback port.
pub fn serve(routes: Vec<(&str, &str, String)>) -> Stub {
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let log = Arc::clone(&seen);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let base = format!("http://{}", listener.local_addr().expect("stub address"));
    let routes: Arc<Routes> = Arc::new(
        routes
            .into_iter()
            .map(|(key, status, body)| (key.to_string(), (status.to_string(), body)))
            .collect(),
    );
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            answer(stream, &routes, &log);
        }
    });
    Stub { base, seen }
}

/// A base URL nothing listens on: bound once to learn a free port, released.
pub fn closed_base() -> String {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    format!("http://{}", l.local_addr().expect("address"))
}

fn answer(stream: TcpStream, routes: &Routes, log: &Mutex<Vec<Seen>>) {
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
    let key = format!("{method} {path}");
    if let Ok(mut seen) = log.lock() {
        seen.push(Seen {
            method,
            target,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        });
    }
    let (status, out) = routes
        .get(&key)
        .or_else(|| routes.get(&path))
        .cloned()
        .unwrap_or_else(|| ("404 Not Found".to_string(), r#"{"error":"not found"}"#.to_string()));
    let mut w = reader.into_inner();
    let _ = write!(
        w,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    );
    let _ = w.flush();
}
