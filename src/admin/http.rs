//! The admin listener's HTTP: read a request, write a reply.

use std::{
    io,
    net::{TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

use super::{Query, Reply, route_with_body};
use crate::{
    app::App,
    http::request::{Limits, Status, parse_request, serve_requests},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// An admin request; the body can be a data import.
const LIMITS: Limits = Limits {
    head: 16 * 1024,
    line: 16 * 1024,
    body: 32 * 1024 * 1024,
};

pub fn serve(listener: TcpListener, state: Arc<App>) -> io::Result<()> {
    tracing::info!(addr = ?listener.local_addr().ok(), "admin API listening");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &state) {
                        tracing::debug!(%error, "admin connection failed");
                    }
                });
            }
            Err(error) => tracing::warn!(%error, "admin accept failed"),
        }
    }
    Ok(())
}

fn handle_connection(stream: TcpStream, state: &App) -> io::Result<()> {
    stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    serve_requests(
        &stream,
        &stream,
        &LIMITS,
        false,
        |request| answer(state, request),
        |status| encode_reply("GET", status_reply(status)),
    )
}

/// The reply to one request, read by the shared parser with the admin's limits.
pub(super) fn answer(state: &App, request: &[u8]) -> Vec<u8> {
    match parse_request(request, &LIMITS) {
        Ok((request, _)) => encode_reply(
            request.method,
            route_with_body(state, request.method, request.target, request.body),
        ),
        Err(status) => encode_reply("GET", status_reply(status)),
    }
}

/// Refused request: bad line, header, `Content-Length`, `Transfer-Encoding`, or too large.
fn status_reply(status: Status) -> Reply {
    Reply::status(match status {
        Status::BadRequest => 400,
        Status::HeadersTooLarge => 431,
        Status::PayloadTooLarge => 413,
    })
}

fn encode_reply(method: &str, reply: Reply) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        reply.code,
        reason(reply.code),
        reply.content_type,
        reply.body.len()
    )
    .into_bytes();
    if method != "HEAD" {
        bytes.extend_from_slice(&reply.body);
    }
    bytes
}

pub(super) fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

pub(super) fn parse_query(query: &str) -> Query {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((percent_decode_query(name)?, percent_decode_query(value)?))
        })
        .collect()
}

fn percent_decode_query(text: &str) -> Option<String> {
    percent_decode(&text.replace('+', " "))
}

pub(super) fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}
