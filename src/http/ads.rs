//! Minimal ad endpoint for both builds.

use std::{
    io,
    net::{TcpListener, TcpStream},
};

use crate::time::Utc;

use super::request::{
    Limits, READ_IDLE_TIMEOUT, empty_response, idle, parse_request, serve_requests, status_response,
};

pub fn serve_ads(listener: TcpListener, service_host: String) -> io::Result<()> {
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let service_host = service_host.clone();
                std::thread::spawn(move || {
                    let served = stream
                        .set_read_timeout(Some(READ_IDLE_TIMEOUT))
                        .and_then(|()| serve_ads_connection(&stream, &service_host));
                    match served {
                        Ok(()) => {}
                        Err(error) if idle(&error) => {
                            tracing::trace!("ads HTTP connection idle: closed");
                        }
                        Err(error) => tracing::warn!(%error, "ads HTTP connection failed"),
                    }
                });
            }
            Err(error) => tracing::warn!(%error, "ads HTTP accept failed"),
        }
    }
    Ok(())
}

/// One ads request on `stream`, answered, then the connection ends.
pub(crate) fn serve_ads_connection(stream: &TcpStream, service_host: &str) -> io::Result<()> {
    serve_requests(
        stream,
        stream,
        &Limits::CONSOLE,
        false,
        |request| handle_ads_request(request, service_host, crate::time::now()),
        status_response,
    )
}

pub(super) fn handle_ads_request(request: &[u8], service_host: &str, now: Utc) -> Vec<u8> {
    match parse_request(request, &Limits::CONSOLE) {
        Ok((request, _))
            if request.method == "POST" && request.path == "/adsrv/5/locateService" =>
        {
            let body = locate_service_message(service_host, now);
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/massive\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            response.extend_from_slice(&body);
            response
        }
        Ok((request, _)) => {
            tracing::warn!(
                method = request.method,
                path = request.path,
                "unknown ads request"
            );
            empty_response(404, "Not Found")
        }
        Err(status) => status_response(status),
    }
}

fn locate_service_message(service_host: &str, now: Utc) -> Vec<u8> {
    let mut payload = vec![0x0d, 0];
    payload.push(0x3b);
    let timestamp = u64::try_from(now.unix_ms()).expect("the host clock is after 1970");
    payload.extend_from_slice(&timestamp.to_be_bytes());
    let host_len = u16::try_from(service_host.len()).expect("service host fits u16");
    for tag in [0x22, 0x2d] {
        payload.push(tag);
        payload.extend_from_slice(&host_len.to_be_bytes());
        payload.extend_from_slice(service_host.as_bytes());
    }
    payload.push(0x3a);
    payload.extend_from_slice(&4u16.to_be_bytes());
    payload.push(0x48);
    payload.extend_from_slice(&host_len.to_be_bytes());
    payload.extend_from_slice(service_host.as_bytes());

    let mut message = vec![5, 0xca];
    message.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("LocateService payload fits u32")
            .to_be_bytes(),
    );
    message.extend_from_slice(&payload);
    message
}
