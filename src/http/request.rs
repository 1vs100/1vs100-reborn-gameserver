//! HTTP/1.1 request loop and strict parser for the Grid, ads, and admin listeners.

use std::{
    io::{self, Read, Write},
    time::Duration,
};

/// The largest request a listener reads.
pub(crate) struct Limits {
    /// The head, request line and headers with their line ends.
    pub head: usize,
    /// One line of the head.
    pub line: usize,
    pub body: usize,
}

impl Limits {
    /// The consoles' Grid and ads requests.
    pub(crate) const CONSOLE: Self = Self {
        head: 8 * 1024,
        line: 2 * 1024,
        body: 64 * 1024,
    };
}
/// How long a console connection may send nothing before it is closed, as on the SBG and
/// matchmaking listeners. Without it a client that connects and sends nothing holds a thread.
pub(super) const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Whether a connection ended because it sent nothing for [`READ_IDLE_TIMEOUT`].
pub(super) fn idle(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Answer each request read from `reader` with `handle`'s reply, written to `writer`, in order.
/// `keep_alive`: the connection carries further requests (the Grid); otherwise it ends after the
/// first reply (ads). Every complete request in the buffer is answered before the next read.
/// What is left when the stream ends goes to `handle` as one request; a
/// request too large or malformed to frame is answered with its status and ends the connection.
/// `refuse`: the reply to a request that cannot be framed.
pub(crate) fn serve_requests(
    mut reader: impl Read,
    mut writer: impl Write,
    limits: &Limits,
    keep_alive: bool,
    mut handle: impl FnMut(&[u8]) -> Vec<u8>,
    refuse: impl Fn(Status) -> Vec<u8>,
) -> io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        match request_len(&buf, limits) {
            Ok(Some(len)) => {
                writer.write_all(&handle(&buf[..len]))?;
                if !keep_alive {
                    return Ok(());
                }
                buf.drain(..len);
                // A further request may already be in the buffer (sent in the same packet).
                continue;
            }
            Ok(None) => {}
            Err(status) => {
                writer.write_all(&refuse(status))?;
                return Ok(());
            }
        }
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            if !buf.is_empty() {
                writer.write_all(&handle(&buf))?;
            }
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..read]);
    }
}

pub(crate) struct Request<'a> {
    pub method: &'a str,
    /// The request target as sent, query included.
    pub target: &'a str,
    /// The target without its query.
    pub path: &'a str,
    /// The `Host` header; `None` when the request carries no usable one.
    pub host: Option<&'a str>,
    pub body: &'a [u8],
}

#[derive(Copy, Clone, Debug)]
pub(crate) enum Status {
    BadRequest,
    HeadersTooLarge,
    PayloadTooLarge,
}

pub(super) fn request_len(bytes: &[u8], limits: &Limits) -> Result<Option<usize>, Status> {
    let Some(header_len) = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
    else {
        return if bytes.len() > limits.head {
            Err(Status::HeadersTooLarge)
        } else {
            Ok(None)
        };
    };
    if header_len > limits.head {
        return Err(Status::HeadersTooLarge);
    }
    let head = std::str::from_utf8(&bytes[..header_len - 4]).map_err(|_| Status::BadRequest)?;
    let mut content_length = 0;
    let mut content_length_seen = false;
    for line in head.split("\r\n").skip(1) {
        if line.len() > limits.line {
            return Err(Status::HeadersTooLarge);
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(Status::BadRequest);
        };
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(Status::BadRequest);
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length_seen {
                return Err(Status::BadRequest);
            }
            content_length = value.trim().parse().map_err(|_| Status::BadRequest)?;
            content_length_seen = true;
        }
    }
    if content_length > limits.body {
        return Err(Status::PayloadTooLarge);
    }
    let total = header_len
        .checked_add(content_length)
        .ok_or(Status::PayloadTooLarge)?;
    Ok((bytes.len() >= total).then_some(total))
}

/// The first request in `bytes`, and how many bytes it takes.
pub(crate) fn parse_request<'a>(
    bytes: &'a [u8],
    limits: &Limits,
) -> Result<(Request<'a>, usize), Status> {
    let consumed = request_len(bytes, limits)?.ok_or(Status::BadRequest)?;
    let header_len = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .ok_or(Status::BadRequest)?;
    let head = std::str::from_utf8(&bytes[..header_len - 4]).map_err(|_| Status::BadRequest)?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or(Status::BadRequest)?;
    if request_line.len() > limits.line {
        return Err(Status::HeadersTooLarge);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(Status::BadRequest)?;
    let target = parts.next().ok_or(Status::BadRequest)?;
    let version = parts.next().ok_or(Status::BadRequest)?;
    if parts.next().is_some()
        || !target.starts_with('/')
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
    {
        return Err(Status::BadRequest);
    }
    let mut host = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(Status::BadRequest)?;
        if name.eq_ignore_ascii_case("host") {
            host = Some(value.trim()).filter(|value| !value.is_empty());
        }
    }
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    Ok((
        Request {
            method,
            target,
            path,
            host,
            body: &bytes[header_len..consumed],
        },
        consumed,
    ))
}

pub(super) fn status_response(status: Status) -> Vec<u8> {
    match status {
        Status::BadRequest => empty_response(400, "Bad Request"),
        Status::HeadersTooLarge => empty_response(431, "Request Header Fields Too Large"),
        Status::PayloadTooLarge => empty_response(413, "Payload Too Large"),
    }
}

pub(super) fn empty_response(code: u16, reason: &str) -> Vec<u8> {
    format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: Close\r\n\r\n")
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A parsed request as one line: what it asks for, its host and whether its body is here.
    fn shown(parsed: Result<(Request<'_>, usize), Status>) -> String {
        match parsed {
            Ok((request, used)) => format!(
                "{} {} (path {}, host {:?}, {} body bytes) takes {used} bytes",
                request.method,
                request.target,
                request.path,
                request.host,
                request.body.len(),
            ),
            Err(status) => format!("{status:?}"),
        }
    }

    #[test]
    fn requests_parse_to_their_parts() {
        let cases: [(&str, &[u8]); 15] = [
            (
                "a Grid sessions get",
                b"GET /grid/v1/sessions/1vs100/en-US/20261023 HTTP/1.1\r\nHost: 192.0.2.10\r\n\r\n",
            ),
            (
                "a get with a query",
                b"GET /grid/v1/sessions/1vs100/en-US/20261023?lang=en-US HTTP/1.1\r\n\
                  Host: grid.example:8080\r\n\r\n",
            ),
            (
                "no host header",
                b"GET /status/v1/live HTTP/1.1\r\nAccept: */*\r\n\r\n",
            ),
            (
                "an empty host",
                b"GET /status/v1/live HTTP/1.1\r\nHost:  \r\n\r\n",
            ),
            (
                "a post with its body",
                b"POST /admin/v1/questions HTTP/1.1\r\nHost: 127.0.0.1:8080\r\n\
                  Content-Length: 5\r\n\r\nhello",
            ),
            (
                "lower-case headers over HTTP/1.0",
                b"GET /x HTTP/1.0\r\nhost: h\r\n\r\n",
            ),
            (
                "a body that is not all here",
                b"POST /admin/v1/questions HTTP/1.1\r\nContent-Length: 9\r\n\r\nhello",
            ),
            ("no head end", b"GET /x HTTP/1.1\r\nHost: h"),
            (
                "a version no listener speaks",
                b"GET /x HTTP/2\r\nHost: h\r\n\r\n",
            ),
            (
                "a target that is no path",
                b"GET * HTTP/1.1\r\nHost: h\r\n\r\n",
            ),
            (
                "a request line of four words",
                b"GET /x HTTP/1.1 extra\r\n\r\n",
            ),
            (
                "two content lengths",
                b"POST /x HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\na",
            ),
            (
                "a transfer encoding",
                b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            ),
            (
                "a header with no colon",
                b"GET /x HTTP/1.1\r\ngarbage\r\n\r\n",
            ),
            (
                "a request that is no text",
                b"GET /\xff HTTP/1.1\r\nHost: h\r\n\r\n",
            ),
        ];
        let shown_lines: Vec<String> = cases
            .into_iter()
            .map(|(what, request)| {
                format!(
                    "{what}: {}",
                    shown(parse_request(request, &Limits::CONSOLE))
                )
            })
            .collect();
        insta::assert_snapshot!(shown_lines.join("\n"));
    }
}
