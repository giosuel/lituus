//! HTTP/1.1 over cupsd's domain socket: one request per connection, the body framed by
//! Content-Length (what cupsd sends) or chunked.

use std::io;
use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// A request's bytes: the head, then the body.
pub(crate) fn encode(method: &str, path: &str, content_type: &str, auth: Option<&str>, body: &[u8]) -> Vec<u8> {
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    if !body.is_empty() || method == "POST" {
        head.push_str(&format!("Content-Type: {content_type}\r\nContent-Length: {}\r\n", body.len()));
    }
    if let Some(auth) = auth {
        head.push_str(&format!("Authorization: {auth}\r\n"));
    }
    head.push_str("\r\n");
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    out
}

/// The status, the headers, and the head's length.
type Head = (u16, Vec<(String, String)>, usize);

/// The status line and headers, when the whole head is in `buf`: (status, headers, the head's length).
fn head(buf: &[u8]) -> Option<Result<Head, Error>> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let text = String::from_utf8_lossy(&buf[..end - 4]);
    let mut lines = text.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.strip_prefix("HTTP/1."))
        .and_then(|l| l.get(2..5))
        .and_then(|s| s.parse::<u16>().ok());
    let Some(status) = status else {
        return Some(Err(Error::Parse("not an HTTP response".into())));
    };
    let headers =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    Some(Ok((status, headers, end)))
}

/// A chunked body, when it's complete in `buf`.
fn dechunk(buf: &[u8]) -> Option<Result<Vec<u8>, Error>> {
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        let line_end = buf[at..].windows(2).position(|w| w == b"\r\n")? + at;
        let size = String::from_utf8_lossy(&buf[at..line_end]);
        let size = size.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size, 16) else {
            return Some(Err(Error::Parse(format!("bad chunk size {size:?}"))));
        };
        let start = line_end + 2;
        if size == 0 {
            return Some(Ok(out));
        }
        if buf.len() < start + size + 2 {
            return None;
        }
        out.extend_from_slice(&buf[start..start + size]);
        at = start + size + 2;
    }
}

/// Reads a whole response from what arrives in `buf` as it grows; `eof` when the peer closed.
pub(crate) fn parse(buf: &[u8], eof: bool) -> Option<Result<Response, Error>> {
    let (status, headers, len) = match head(buf) {
        None if eof => return Some(Err(Error::Parse("the connection closed before a response".into()))),
        None => return None,
        Some(Err(e)) => return Some(Err(e)),
        Some(Ok(h)) => h,
    };
    // An interim 100 Continue: the real response follows.
    if (100..200).contains(&status) {
        return parse(&buf[len..], eof);
    }
    let get = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    let rest = &buf[len..];
    let body = if get("Transfer-Encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        match dechunk(rest) {
            None if eof => return Some(Err(Error::Parse("a chunked body was cut short".into()))),
            None => return None,
            Some(b) => b,
        }
    } else if let Some(n) = get("Content-Length").and_then(|v| v.parse::<usize>().ok()) {
        if rest.len() < n {
            if eof {
                return Some(Err(Error::Parse("the body was cut short".into())));
            }
            return None;
        }
        Ok(rest[..n].to_vec())
    } else if eof {
        Ok(rest.to_vec())
    } else {
        return None;
    };
    Some(body.map(|body| Response { status, headers, body }))
}

fn not_running(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
}

/// Sends one request and reads its response, within `limit`.
pub(crate) async fn send(socket: &Path, request: &[u8], limit: Duration) -> Result<Response, Error> {
    let go = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| if not_running(&e) { Error::NotRunning } else { Error::Io(e) })?;
        // cupsd may answer (401) before it has read a long body and close: its answer still counts.
        let wrote = stream.write_all(request).await;
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 16384];
        loop {
            let n = match stream.read(&mut chunk).await {
                Ok(n) => n,
                // A reset after a whole answer (cupsd closing on an unread body): read what came.
                Err(_) if !buf.is_empty() => 0,
                Err(e) => return Err(Error::Io(e)),
            };
            buf.extend_from_slice(&chunk[..n]);
            if let Some(done) = parse(&buf, n == 0) {
                return done;
            }
            if n == 0 {
                wrote.map_err(Error::Io)?;
                return Err(Error::Parse("no response".into()));
            }
        }
    };
    tokio::time::timeout(limit, go).await.map_err(|_| Error::Timeout)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_has_its_length_and_the_authorization_when_given() {
        let r = String::from_utf8(encode("POST", "/admin/", "application/ipp", Some("PeerCred anna"), b"abc")).unwrap();
        assert!(r.starts_with("POST /admin/ HTTP/1.1\r\nHost: localhost\r\n"), "{r}");
        assert!(r.contains("Content-Length: 3\r\n"), "{r}");
        assert!(r.contains("Authorization: PeerCred anna\r\n"), "{r}");
        assert!(r.ends_with("\r\n\r\nabc"), "{r}");
        let g = String::from_utf8(encode("GET", "/printers/x.ppd", "", None, b"")).unwrap();
        assert!(!g.contains("Content-Length") && !g.contains("Authorization"), "{g}");
    }

    #[test]
    fn a_response_is_whole_only_when_its_body_is() {
        let full = b"HTTP/1.1 200 OK\r\nServer: CUPS/2.4 IPP/2.1\r\nContent-Length: 5\r\n\r\nhello";
        assert!(parse(&full[..30], false).is_none());
        assert!(parse(&full[..full.len() - 1], false).is_none());
        let r = parse(full, false).unwrap().unwrap();
        assert_eq!((r.status, r.body.as_slice(), r.header("server")), (200, &b"hello"[..], Some("CUPS/2.4 IPP/2.1")));
        assert!(matches!(parse(&full[..full.len() - 1], true), Some(Err(Error::Parse(_)))));
    }

    #[test]
    fn chunked_bodies_and_interim_responses_are_read() {
        let r = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n";
        let got = parse(r, false).unwrap().unwrap();
        assert_eq!((got.status, got.body), (200, b"abcde".to_vec()));
        assert!(parse(&r[..r.len() - 8], false).is_none());
    }

    #[test]
    fn a_refusal_keeps_its_challenge() {
        let r = b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"CUPS\", PeerCred, Local trc=\"y\"\r\nContent-Length: 0\r\n\r\n";
        let got = parse(r, false).unwrap().unwrap();
        assert_eq!(got.status, 401);
        assert!(got.header("www-authenticate").unwrap().contains("PeerCred"));
    }

    #[tokio::test]
    async fn no_socket_is_not_running() {
        let dir = std::env::temp_dir().join(format!("lituus-print-none-{}", std::process::id()));
        let got = send(&dir.join("cups.sock"), b"GET / HTTP/1.1\r\n\r\n", Duration::from_secs(1)).await;
        assert!(matches!(got, Err(Error::NotRunning)), "{got:?}");
    }
}
