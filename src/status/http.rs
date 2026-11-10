//! The status listener's HTTP. It faces the public internet, so requests are small, connections
//! are short and their number is capped.

use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use super::{Reply, Status, route};
use crate::http::request::{Limits, Status as Refused, parse_request, serve_requests};

/// For the whole request, however slowly it arrives.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Connections served at once; more are answered 503.
const MAX_CONNECTIONS: usize = 64;
/// The 503 is written by the accepting thread, so it may not wait on a slow client.
const BUSY_WRITE_TIMEOUT: Duration = Duration::from_millis(100);
/// GET requests only: no body.
const LIMITS: Limits = Limits {
    head: 8 * 1024,
    line: 2 * 1024,
    body: 0,
};

pub fn serve(listener: TcpListener, status: Status) -> io::Result<()> {
    tracing::info!(addr = ?listener.local_addr().ok(), "status API listening");
    let status = Arc::new(status);
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%error, "status accept failed");
                continue;
            }
        };
        if open.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::AcqRel);
            tracing::debug!("status connection refused: too many open");
            let busy = encode("GET", Reply::error(503, "too many connections"));
            let _ = stream
                .set_write_timeout(Some(BUSY_WRITE_TIMEOUT))
                .and_then(|()| (&stream).write_all(&busy));
            continue;
        }
        let (status, open) = (Arc::clone(&status), Arc::clone(&open));
        std::thread::spawn(move || {
            if let Err(error) = handle_connection(stream, &status) {
                tracing::debug!(%error, "status connection failed");
            }
            open.fetch_sub(1, Ordering::AcqRel);
        });
    }
    Ok(())
}

fn handle_connection(stream: TcpStream, status: &Status) -> io::Result<()> {
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    serve_requests(
        Deadline {
            stream: &stream,
            until: Instant::now() + REQUEST_TIMEOUT,
        },
        &stream,
        &LIMITS,
        false,
        |request| answer(status, request),
        |refused| encode("GET", refused_reply(refused)),
    )
}

/// Reads that fail once `until` has passed.
struct Deadline<'a> {
    stream: &'a TcpStream,
    until: Instant,
}

impl Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(buf)
    }
}

fn answer(status: &Status, request: &[u8]) -> Vec<u8> {
    match parse_request(request, &LIMITS) {
        Ok((request, _)) => encode(
            request.method,
            route(status, request.method, request.target),
        ),
        Err(refused) => encode("GET", refused_reply(refused)),
    }
}

fn refused_reply(refused: Refused) -> Reply {
    match refused {
        Refused::BadRequest => Reply::error(400, "bad request"),
        Refused::HeadersTooLarge => Reply::error(431, "request headers too large"),
        Refused::PayloadTooLarge => Reply::error(413, "a status request has no body"),
    }
}

fn encode(method: &str, reply: Reply) -> Vec<u8> {
    let cache = match reply.max_age_secs {
        Some(secs) => format!("public, max-age={secs}"),
        None => "no-store".to_owned(),
    };
    let allow = if reply.code == 405 {
        "Allow: GET, HEAD\r\n"
    } else {
        ""
    };
    // Anyone may show the public status on a page of their own.
    let mut bytes = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Cache-Control: {cache}\r\n{allow}Access-Control-Allow-Origin: *\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        reply.code,
        reason(reply.code),
        reply.body.len(),
    )
    .into_bytes();
    if method != "HEAD" {
        bytes.extend_from_slice(&reply.body);
    }
    bytes
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(request: &str) -> String {
        String::from_utf8(answer(&super::super::tests::status(), request.as_bytes())).unwrap()
    }

    #[test]
    fn replies_carry_their_cache_time_and_head_has_no_body() {
        let get = exchange("GET /status/v1/server HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(get.starts_with("HTTP/1.1 200 OK\r\n"), "{get}");
        assert!(
            get.contains("Cache-Control: public, max-age=30\r\n"),
            "{get}"
        );
        assert!(get.ends_with('}'), "{get}");
        let head = exchange("HEAD /status/v1/server HTTP/1.1\r\nHost: x\r\n\r\n");
        let length = |reply: &str| {
            reply
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .map(str::to_owned)
        };
        assert!(head.ends_with("\r\n\r\n"), "{head}");
        assert_eq!(length(&head), length(&get));
    }

    #[test]
    fn requests_that_are_not_reads_are_refused() {
        let post = exchange("POST /status/v1/server HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(post.starts_with("HTTP/1.1 405 ") && post.contains("Allow: GET, HEAD\r\n"));
        assert!(post.contains("Cache-Control: no-store\r\n"), "{post}");
        let body = exchange("GET /status/v1/server HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi");
        assert!(body.starts_with("HTTP/1.1 413 "), "{body}");
        let long = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(3000));
        assert!(exchange(&long).starts_with("HTTP/1.1 431 "));
    }
}
