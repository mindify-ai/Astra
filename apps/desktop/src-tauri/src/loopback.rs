//! The desktop's half of one-click sign-in to a sync server: a loopback redirect (RFC 8252).
//!
//! The app opens GitHub's sign-in page in the system browser. GitHub sends the browser to the relay, and the
//! relay sends it on to `http://127.0.0.1:<port>/callback?astraCode=…` (or `?authError=…`), where this listener
//! waits. It answers with a page that says to go back to Astra, hands the query to the app, and stops.
//!
//! The query carries no secret on its own: the relay only trades the one-time code for a session together with
//! the PKCE verifier, which never leaves the webview. Std only, so it is tested without the Tauri toolchain.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// The path the relay is told to return to.
pub const CALLBACK_PATH: &str = "/callback";

/// Starts listening on a free loopback port and returns it. `on_result(port, query)` runs once, on this
/// listener's thread: with the callback's query, or with `authError=expired` when nothing arrives in `timeout`.
pub fn listen(
    timeout: Duration,
    on_result: impl FnOnce(u16, String) + Send + 'static,
) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    std::thread::Builder::new()
        .name("astra-oauth-loopback".into())
        .spawn(move || {
            let deadline = Instant::now() + timeout;
            let query = loop {
                if Instant::now() >= deadline {
                    break "authError=expired".to_string();
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Some(q) = serve(stream) {
                            break q;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            };
            on_result(port, query);
        })?;
    Ok(port)
}

/// Answers one request; returns the query when it is the sign-in result.
fn serve(mut stream: TcpStream) -> Option<String> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let query = result_query(head.lines().next().unwrap_or_default());
    let (status, body) = match &query {
        Some(q) if q.split('&').any(|kv| kv.starts_with("astraCode=")) => ("200 OK", page(true)),
        Some(_) => ("200 OK", page(false)),
        None => ("404 Not Found", String::from("Not found")),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
    query
}

/// The query of `GET /callback?…` when it carries a sign-in result (`astraCode` or `authError`).
pub fn result_query(request_line: &str) -> Option<String> {
    let mut parts = request_line.split_whitespace();
    if parts.next() != Some("GET") {
        return None;
    }
    let (path, query) = parts.next()?.split_once('?')?;
    if path != CALLBACK_PATH {
        return None;
    }
    let has_result = query
        .split('&')
        .any(|kv| kv.starts_with("astraCode=") || kv.starts_with("authError="));
    has_result.then(|| query.to_string())
}

fn page(ok: bool) -> String {
    let (title, text) = if ok {
        ("Signed in", "You're signed in. Astra is back in front; you can close this tab.")
    } else {
        ("Sign-in didn't finish", "Sign-in didn't finish. Go back to Astra to see why, or try again.")
    };
    format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
         <title>Astra · {title}</title>\
         <body style=\"font:16px/1.6 system-ui,sans-serif;background:#000;color:#fff;padding:48px\">\
         <p style=\"letter-spacing:.22em;font-size:13px\">ASTRA</p><h1 style=\"font-size:22px\">{title}</h1>\
         <p style=\"color:#a3a3a3\">{text}</p><script>setTimeout(()=>window.close(),800)</script></body>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn reads_only_the_callback_result() {
        assert_eq!(
            result_query("GET /callback?astraCode=abc HTTP/1.1").as_deref(),
            Some("astraCode=abc")
        );
        assert_eq!(
            result_query("GET /callback?x=1&authError=denied HTTP/1.1").as_deref(),
            Some("x=1&authError=denied")
        );
        assert_eq!(result_query("GET /favicon.ico HTTP/1.1"), None);
        assert_eq!(result_query("GET /callback HTTP/1.1"), None);
        assert_eq!(result_query("GET /callback?other=1 HTTP/1.1"), None);
        assert_eq!(result_query("POST /callback?astraCode=abc HTTP/1.1"), None);
        assert_eq!(result_query("GET /elsewhere?astraCode=abc HTTP/1.1"), None);
    }

    fn get(port: u16, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").expect("write");
        let mut out = String::new();
        s.read_to_string(&mut out).expect("read");
        out
    }

    #[test]
    fn hands_over_the_result_once_and_answers_the_browser() {
        let (tx, rx) = mpsc::channel();
        let port = listen(Duration::from_secs(10), move |p, q| {
            tx.send((p, q)).expect("send");
        })
        .expect("listen");
        // A browser asking for its icon first does not end the wait.
        assert!(get(port, "/favicon.ico").starts_with("HTTP/1.1 404"));
        let page = get(port, "/callback?astraCode=one-time");
        assert!(page.starts_with("HTTP/1.1 200 OK"));
        assert!(page.contains("Cache-Control: no-store"));
        assert!(page.contains("You're signed in"));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).expect("result"),
            (port, "astraCode=one-time".to_string())
        );
        // Then it stops listening.
        std::thread::sleep(Duration::from_millis(100));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn gives_up_after_the_timeout() {
        let (tx, rx) = mpsc::channel();
        let port = listen(Duration::from_millis(200), move |p, q| {
            tx.send((p, q)).expect("send");
        })
        .expect("listen");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).expect("result"),
            (port, "authError=expired".to_string())
        );
    }
}
