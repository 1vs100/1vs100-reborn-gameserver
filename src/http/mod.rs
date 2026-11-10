use std::{
    io::{self, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use rsa::RsaPrivateKey;

use crate::{
    grid::{self, GridConfig},
    schedule::Schedule,
    sync::lock,
    wire::Protocol,
};

mod ads;
mod livehost;
pub(crate) mod request;

pub use ads::serve_ads;
pub(crate) use ads::serve_ads_connection;
pub use livehost::{LivehostError, LivehostStream};
use request::{
    Limits, READ_IDLE_TIMEOUT, Request, empty_response, idle, parse_request, serve_requests,
    status_response,
};

/// What the Grid is served with: the signing key, the title build, the content ids and the
/// schedule the listed shows come from.
pub struct GridService {
    pub key: RsaPrivateKey,
    pub protocol: Protocol,
    pub config: GridConfig,
    pub schedule: Arc<Mutex<Schedule>>,
    pub livehost: LivehostStream,
}

pub fn handle_request(request: &[u8], now: SystemTime, service: &GridService) -> Vec<u8> {
    match parse_request(request, &Limits::CONSOLE) {
        Ok((request, _)) => route(request, now, service),
        Err(status) => {
            tracing::debug!(?status, "rejected malformed HTTP request");
            status_response(status)
        }
    }
}

/// Grid and livehost HTTP; the Grid document is the one of the build this server serves.
pub fn serve(listener: TcpListener, service: GridService) -> io::Result<()> {
    serve_with_read_timeout(listener, service, READ_IDLE_TIMEOUT)
}

fn serve_with_read_timeout(
    listener: TcpListener,
    service: GridService,
    read_timeout: Duration,
) -> io::Result<()> {
    let service = Arc::new(service);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let service = Arc::clone(&service);
                std::thread::spawn(move || {
                    let served = stream
                        .set_read_timeout(Some(read_timeout))
                        .and_then(|()| stream.try_clone())
                        .and_then(|reader| {
                            handle_connection_with_clock(reader, &stream, SystemTime::now, &service)
                        });
                    match served {
                        Ok(()) => {}
                        Err(error) if idle(&error) => {
                            tracing::trace!("HTTP connection idle: closed");
                        }
                        Err(error) => tracing::warn!(%error, "HTTP connection failed"),
                    }
                });
            }
            Err(error) => tracing::warn!(%error, "HTTP accept failed"),
        }
    }
    Ok(())
}

/// The Grid connection: requests in order on one connection (keep-alive), each answered at the
/// time `now` gives.
fn handle_connection_with_clock<R: Read, W: Write, F: FnMut() -> SystemTime>(
    reader: R,
    writer: W,
    mut now: F,
    service: &GridService,
) -> io::Result<()> {
    serve_requests(
        reader,
        writer,
        &Limits::CONSOLE,
        true,
        |request| handle_request(request, now(), service),
        status_response,
    )
}

fn route(request: Request<'_>, now: SystemTime, service: &GridService) -> Vec<u8> {
    let GridService {
        key,
        protocol,
        config,
        schedule,
        livehost,
    } = service;
    let protocol = *protocol;
    tracing::trace!(
        method = request.method,
        path = request.path,
        host = ?request.host,
        "HTTP request"
    );
    let Some(host) = request.host else {
        tracing::debug!("HTTP request without Host header");
        return empty_response(400, "Bad Request");
    };
    if request.method != "GET" {
        tracing::warn!(
            method = request.method,
            path = request.path,
            "unsupported HTTP method"
        );
        return empty_response(404, "Not Found");
    }
    if request.path == "/livehost.wma" {
        tracing::info!("serving livehost stream");
        let stream = livehost.bytes();
        // The whole stream at once; the player buffers it and plays at its own pace.
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-mms-framed\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            stream.len()
        )
        .into_bytes();
        response.extend_from_slice(stream);
        return response;
    }
    let route_path = request.path.strip_prefix('/').unwrap_or(request.path);
    let parts: Vec<_> = route_path.split('/').collect();
    if parts.len() != 6
        || parts.iter().any(|part| part.is_empty())
        || parts[0] != "grid"
        || parts[1] != "v1"
        || parts[2] != "sessions"
    {
        tracing::warn!(path = request.path, "unknown HTTP route");
        return empty_response(404, "Not Found");
    }
    tracing::trace!(path = request.path, ?protocol, "grid sessions request");
    let xml = {
        let mut schedule = lock(schedule);
        match protocol {
            Protocol::V1149 => {
                grid::sessions_xml_1149(config, &mut schedule, parts[3], parts[5], now, host)
            }
            Protocol::V1317 => grid::sessions_xml(config, &mut schedule, parts[3], parts[5], now),
        }
    };
    grid::signed_response(request.path, host, xml.as_bytes(), now, key).unwrap_or_else(|error| {
        tracing::error!(?error, path = request.path, "failed to sign grid response");
        empty_response(500, "Internal Server Error")
    })
}

#[cfg(test)]
mod tests {
    use super::{ads::handle_ads_request, request::request_len, *};

    /// The Grid served with the test key and no schedule.
    fn service(protocol: Protocol) -> GridService {
        GridService {
            key: key(),
            protocol,
            config: GridConfig::default(),
            schedule: Default::default(),
            livehost: LivehostStream::load(None).unwrap(),
        }
    }
    use flate2::read::GzDecoder;
    use rsa::{RsaPrivateKey, pkcs1::DecodeRsaPrivateKey};
    use std::{io::Read, time::Duration};

    fn key() -> RsaPrivateKey {
        RsaPrivateKey::from_pkcs1_pem(include_str!("../../keys/grid_lan_private.pem")).unwrap()
    }

    fn fixed_now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_510_400)
    }

    fn split_response(response: &[u8]) -> (&str, &[u8]) {
        let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (
            std::str::from_utf8(&response[..split]).unwrap(),
            &response[split + 4..],
        )
    }

    fn header<'a>(headers: &'a str, name: &str) -> &'a str {
        headers
            .lines()
            .find_map(|line| line.strip_prefix(name).map(str::trim))
            .unwrap()
    }

    fn grid_request(target: &str) -> Vec<u8> {
        format!(
            "GET {target} HTTP/1.1\r\nHost: 192.0.2.10\r\nAccept-Encoding: gzip\r\nConnection: Keep-alive\r\n\r\n"
        )
        .into_bytes()
    }

    fn grid_xml(protocol: Protocol, user_agent: &str) -> String {
        let request = format!(
            "GET /grid/v1/sessions/1vs100/en-GB/20260927 HTTP/1.1\r\nHost: 192.0.2.10\r\nUser-Agent: {user_agent}\r\n\r\n"
        );
        let response = handle_request(request.as_bytes(), fixed_now(), &service(protocol));
        let (headers, body) = split_response(&response);
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
        let mut xml = String::new();
        GzDecoder::new(body).read_to_string(&mut xml).unwrap();
        xml
    }

    #[test]
    fn grid_document_follows_the_configured_protocol_not_the_user_agent() {
        // `<TitleType>` is only in the v2.0.1149.0 document (`grid::sessions_xml_1149`).
        let xbox_1149_agent = "Xbox Live Game Client/2.0.8508.0";
        assert!(grid_xml(Protocol::V1149, "").contains("<TitleType>"));
        assert!(grid_xml(Protocol::V1149, "other").contains("<TitleType>"));
        assert!(!grid_xml(Protocol::V1317, xbox_1149_agent).contains("<TitleType>"));
    }

    #[test]
    fn locate_service_waits_for_body_and_returns_protocol_response() {
        let head = b"POST /adsrv/5/locateService HTTP/1.1\r\nHost:\r\nContent-Type: application/massive\r\nContent-Length: 68\r\n\r\n";
        assert!(matches!(request_len(head, &Limits::CONSOLE), Ok(None)));
        let mut request = head.to_vec();
        request.extend([0; 68]);
        assert!(
            matches!(request_len(&request, &Limits::CONSOLE), Ok(Some(len)) if len == request.len())
        );

        let response = handle_ads_request(&request, "192.0.2.10", fixed_now().into());
        let (headers, body) = split_response(&response);
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert_eq!(header(headers, "Content-Type:"), "application/massive");
        assert_eq!(header(headers, "Content-Length:"), body.len().to_string());
        assert_eq!(&body[..2], &[5, 0xca]);
        assert_eq!(
            u32::from_be_bytes(body[2..6].try_into().unwrap()) as usize,
            body.len() - 6
        );
        assert_eq!(
            body.windows(b"192.0.2.10".len())
                .filter(|window| *window == b"192.0.2.10")
                .count(),
            3
        );
    }

    #[test]
    fn livehost_get_serves_mms_framed_wma() {
        let request =
            b"GET /livehost.wma HTTP/1.1\r\nHost: 192.0.2.10\r\nContent-Length: 3\r\n\r\n\r\n\0";
        assert!(
            matches!(request_len(request, &Limits::CONSOLE), Ok(Some(len)) if len == request.len())
        );

        let response = handle_request(request, fixed_now(), &service(Protocol::V1317));
        let (headers, body) = split_response(&response);
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert_eq!(header(headers, "Content-Type:"), "application/x-mms-framed");
        assert_eq!(header(headers, "Content-Length:"), body.len().to_string());
        assert_eq!(&body[..2], b"\xA4H");
        let header_packet_len = u16::from_le_bytes([body[2], body[3]]) as usize;
        assert_eq!(
            &body[4 + header_packet_len..6 + header_packet_len],
            b"\xA4D"
        );
        assert!(body.ends_with(b"\x24E\x04\x00\0\0\0\0"));
    }

    #[test]
    fn grid_route_strips_query_from_routing_and_signed_urn() {
        let response = handle_request(
            &grid_request("/grid/v1/sessions/1vs100/en-US/20260927?cache=1"),
            fixed_now(),
            &service(Protocol::V1317),
        );
        let (headers, body) = split_response(&response);

        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert_eq!(
            header(headers, "X-HTTPSig-Urn:"),
            "192.0.2.10/grid/v1/sessions/1vs100/en-US/20260927"
        );
        assert_eq!(header(headers, "Content-Length:"), body.len().to_string());

        let mut xml = String::new();
        GzDecoder::new(body).read_to_string(&mut xml).unwrap();
        assert!(xml.contains("<ChannelName>1vs100</ChannelName>"));
        assert_eq!(xml.matches("<Package>").count(), 3);
    }

    #[test]
    fn double_leading_slash_does_not_match_grid_route() {
        let response = handle_request(
            &grid_request("//grid/v1/sessions/1vs100/en-US/20260927"),
            fixed_now(),
            &service(Protocol::V1317),
        );

        assert!(
            std::str::from_utf8(&response)
                .unwrap()
                .starts_with("HTTP/1.1 404 Not Found\r\n")
        );
    }

    #[test]
    fn a_request_without_a_host_is_answered_as_before() {
        let no_host = b"GET /grid/v1/sessions/1vs100/en-US/20260927 HTTP/1.1\r\n\r\n";
        let (request, _) = parse_request(no_host, &Limits::CONSOLE).unwrap();
        assert_eq!(request.host, None);

        let empty_host = b"GET /grid/v1/sessions/1vs100/en-US/20260927 HTTP/1.1\r\nHost:\r\n\r\n";
        let (request, _) = parse_request(empty_host, &Limits::CONSOLE).unwrap();
        assert_eq!(request.host, None);

        for request in [no_host.as_slice(), empty_host.as_slice()] {
            let response = handle_request(request, fixed_now(), &service(Protocol::V1317));
            assert!(
                std::str::from_utf8(&response)
                    .unwrap()
                    .starts_with("HTTP/1.1 400 Bad Request\r\n")
            );
        }
    }

    #[test]
    fn unknown_or_bad_requests_return_bounded_error_responses() {
        let unknown = handle_request(
            &grid_request("/missing"),
            fixed_now(),
            &service(Protocol::V1317),
        );
        assert!(
            std::str::from_utf8(&unknown)
                .unwrap()
                .starts_with("HTTP/1.1 404 Not Found\r\n")
        );

        let empty_date = handle_request(
            &grid_request("/grid/v1/sessions/1vs100/en-US/"),
            fixed_now(),
            &service(Protocol::V1317),
        );
        assert!(
            std::str::from_utf8(&empty_date)
                .unwrap()
                .starts_with("HTTP/1.1 404 Not Found\r\n")
        );

        let malformed = handle_request(b"nonsense\r\n\r\n", fixed_now(), &service(Protocol::V1317));
        assert!(
            std::str::from_utf8(&malformed)
                .unwrap()
                .starts_with("HTTP/1.1 400 Bad Request\r\n")
        );

        let oversized = format!(
            "GET /grid/v1/sessions/1vs100/en-US/20260927 HTTP/1.1\r\nHost: {}\r\n\r\n",
            "x".repeat(9000)
        );
        let oversized =
            handle_request(oversized.as_bytes(), fixed_now(), &service(Protocol::V1317));
        assert!(
            std::str::from_utf8(&oversized)
                .unwrap()
                .starts_with("HTTP/1.1 431 Request Header Fields Too Large\r\n")
        );
    }

    #[test]
    fn persistent_connection_writes_complete_responses_in_request_order() {
        let mut requests = grid_request("/grid/v1/sessions/one/en-US/20260927");
        requests.extend_from_slice(&grid_request("/grid/v1/sessions/two/en-US/20260927"));
        let mut output = Vec::new();

        handle_connection_with_clock(
            &requests[..],
            &mut output,
            fixed_now,
            &service(Protocol::V1317),
        )
        .unwrap();

        let first_len = response_len(&output);
        let (first_headers, first_body) = split_response(&output[..first_len]);
        let (second_headers, second_body) = split_response(&output[first_len..]);
        assert!(first_headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(second_headers.starts_with("HTTP/1.1 200 OK\r\n"));

        let mut first_xml = String::new();
        GzDecoder::new(first_body)
            .read_to_string(&mut first_xml)
            .unwrap();
        let mut second_xml = String::new();
        GzDecoder::new(second_body)
            .read_to_string(&mut second_xml)
            .unwrap();
        assert!(first_xml.contains("<ChannelName>one</ChannelName>"));
        assert!(second_xml.contains("<ChannelName>two</ChannelName>"));
    }

    #[test]
    fn a_connection_that_sends_nothing_is_closed_after_the_idle_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            serve_with_read_timeout(
                listener,
                service(Protocol::V1317),
                Duration::from_millis(200),
            )
        });
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let started = std::time::Instant::now();
        let mut byte = [0; 1];
        assert_eq!(
            client.read(&mut byte).unwrap(),
            0,
            "the server closes the idle connection"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// Gives its bytes in one read, then fails as a socket read timeout does.
    struct OnePacket(Option<Vec<u8>>);

    impl Read for OnePacket {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let packet = self.0.take().ok_or(io::ErrorKind::TimedOut)?;
            buf[..packet.len()].copy_from_slice(&packet);
            Ok(packet.len())
        }
    }

    #[test]
    fn requests_sent_in_one_packet_are_all_answered_before_the_next_read() {
        let mut packet = grid_request("/grid/v1/sessions/one/en-US/20260927");
        packet.extend_from_slice(&grid_request("/grid/v1/sessions/two/en-US/20260927"));
        let mut output = Vec::new();

        let ended = handle_connection_with_clock(
            OnePacket(Some(packet)),
            &mut output,
            fixed_now,
            &service(Protocol::V1317),
        );

        assert_eq!(ended.unwrap_err().kind(), io::ErrorKind::TimedOut);
        let first_len = response_len(&output);
        assert!(output.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert!(
            output[first_len..].starts_with(b"HTTP/1.1 200 OK\r\n"),
            "the second request waited for another read"
        );
    }

    fn response_len(bytes: &[u8]) -> usize {
        let (headers, body) = split_response(bytes);
        let len: usize = header(headers, "Content-Length:").parse().unwrap();
        bytes.len() - body.len() + len
    }
}
