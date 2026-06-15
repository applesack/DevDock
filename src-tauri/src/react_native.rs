use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use anyhow::{Context, Result, bail};

use crate::config::ServiceConfig;

const DEFAULT_METRO_PORT: u16 = 8081;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

pub fn reload(service: &ServiceConfig) -> Result<()> {
    let port = metro_port(service)?;
    send_reload(port)
}

fn metro_port(service: &ServiceConfig) -> Result<u16> {
    if let Some(value) = service.env.get("RCT_METRO_PORT") {
        return parse_port(value, "RCT_METRO_PORT");
    }

    let mut args = service.args.iter();
    while let Some(arg) = args.next() {
        if arg == "--port" {
            let value = args.next().context("--port requires a value")?;
            return parse_port(value, "--port");
        }
        if let Some(value) = arg.strip_prefix("--port=") {
            return parse_port(value, "--port");
        }
    }

    Ok(DEFAULT_METRO_PORT)
}

fn parse_port(value: &str, source: &str) -> Result<u16> {
    value
        .parse()
        .with_context(|| format!("invalid Metro port '{value}' from {source}"))
}

fn send_reload(port: u16) -> Result<()> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, REQUEST_TIMEOUT)
        .with_context(|| format!("failed to connect to Metro on {address}"))?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;

    write!(
        stream,
        "GET /reload HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .context("failed to send reload request to Metro")?;
    stream.flush().context("failed to flush Metro request")?;

    let mut status = String::new();
    BufReader::new(stream)
        .read_line(&mut status)
        .context("failed to read Metro reload response")?;
    let status = status.trim();
    if !matches!(
        status.split_whitespace().nth(1),
        Some(code) if code.starts_with('2')
    ) {
        bail!("Metro reload request failed: {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, io::Read, net::TcpListener, thread};

    use super::*;

    fn service(args: &[&str], env: &[(&str, &str)]) -> ServiceConfig {
        serde_json::from_value(serde_json::json!({
            "id": "metro",
            "name": "Metro",
            "type": "react-native",
            "group": "mobile",
            "command": "npx",
            "args": args,
            "env": env.iter().copied().collect::<HashMap<_, _>>()
        }))
        .unwrap()
    }

    #[test]
    fn detects_metro_port_from_supported_sources() {
        assert_eq!(
            metro_port(&service(&["react-native", "start", "--port", "8090"], &[])).unwrap(),
            8090
        );
        assert_eq!(
            metro_port(&service(&["react-native", "start", "--port=8091"], &[])).unwrap(),
            8091
        );
        assert_eq!(
            metro_port(&service(
                &["react-native", "start", "--port", "8090"],
                &[("RCT_METRO_PORT", "8092")]
            ))
            .unwrap(),
            8092
        );
        assert_eq!(
            metro_port(&service(&["react-native", "start"], &[])).unwrap(),
            DEFAULT_METRO_PORT
        );
    }

    #[test]
    fn reload_calls_metro_http_endpoint() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 128];
            while !request.ends_with(b"\r\n\r\n") {
                let length = stream.read(&mut buffer).unwrap();
                if length == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..length]);
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("GET /reload HTTP/1.1\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                .unwrap();
        });

        send_reload(port).unwrap();
        server.join().unwrap();
    }
}
