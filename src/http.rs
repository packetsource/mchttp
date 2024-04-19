use std::borrow::Cow;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::ToSocketAddrs;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::task::{spawn, JoinHandle};

use anyhow::{Error, Result};
use lazy_static::lazy_static;
use regex::Regex;

use crate::*;

#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub version: String,
    pub stream: TcpStream,
    pub headers: HashMap<String, String>,
    pub query: HashMap<String, String>,
}

// Main HTTP listener. Binds to port and spawns a new task calling process()
// for each incoming request
pub async fn http_listener<A: ToSocketAddrs + ?Sized>(addr: &A) -> Result<()> {
    // pub async fn http_listener<A: ToSocketAddrs>(addr: &A) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;

    loop {
        let (socket, addr) = listener.accept().await?;

        if CONFIG.verbose {
            eprintln!("HTTP: {:?} connected on FD {}", &addr, &socket.as_raw_fd());
        }
        spawn(async move {
            match process(socket).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Error encountered while handling request: {e}");
                }
            }
            if CONFIG.verbose {
                eprintln!("HTTP: {:?} closed", &addr);
            }
            anyhow::Ok(())
        });
    }

    #[allow(unreachable_code)]
    Ok(())
}

// Entry point per HTTP client connection.
// Read HTTP request and create HTTPRequest structure appropriately
// Then call request_handler() to service the request
pub async fn process(stream: TcpStream) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line_count: u64 = 0;

    let mut method = String::new();
    let mut url = String::new();
    let mut version = String::new();
    let mut headers = HashMap::<String, String>::new();
    let mut query = HashMap::<String, String>::new();

    // Read the header
    loop {
        let mut buf = Vec::<u8>::new();
        let bytes_read = reader.read_until('\n' as u8, &mut buf).await?;

        if bytes_read == 0 {
            return Err(Error::msg("HTTP client EOF"));
        }

        // \r\n at least
        if bytes_read > 2 {
            // Convert binary to UTF8 valid string or fail hard / early return
            let mut line = String::from_utf8(buf)?;

            // Break off line endings please
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }

            // First line is the main verb, URL request and version
            if line_count == 0 {
                if CONFIG.verbose {
                    eprintln!("HTTP: request: {}", &line);
                }
                let verb_tokens: Vec<&str> = line.split(" ").collect();
                if verb_tokens.len() != 3 {
                    return Err(Error::msg(
                        "Malformed HTTP request (method/request/version)",
                    ));
                }
                method = verb_tokens[0].to_lowercase();
                (url, query) = query_string(&verb_tokens[1]);
                version = verb_tokens[2].to_string();

            // Remaining lines are key-value pair headers
            } else {
                match line
                    .split_once(":")
                    .map(|x| (x.0.to_lowercase(), x.1.trim().to_string()))
                {
                    Some((k, v)) => {
                        headers.insert(k, v);
                    }
                    _ => {}
                }
            }

            line_count += 1;
        } else {
            break; // EOF (0) or empty line denoting end of headers (1)
        }
    }

    let http_request = HttpRequest {
        method,
        url,
        version,
        stream: reader.into_inner(),
        headers,
        query,
    };

    // dbg!(&http_request);

    //    Ok(request_handler(http_request).await?)
    request_handler(http_request).await
}

// Utility function to break out query string into constituent KV pairs
pub fn query_string(url: &str) -> (String, HashMap<String, String>) {
    let mut query = HashMap::<String, String>::new();
    let mut bare_url = url.to_string();

    match urlencoding::decode(url) {
        Ok(url) => {
            match url.split_once("?") {
                Some((url, query_string)) => {
                    bare_url = url.to_string(); // re-assign

                    // Split on & to get the individual key-value pairs
                    for kv in query_string.split("&") {
                        // Split on = to separate key and value.
                        // Lowercase the key, and store
                        match kv
                            .split_once("=")
                            .map(|x| (x.0.to_lowercase(), x.1.to_string()))
                        {
                            Some((k, v)) => {
                                query.insert(k, v);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }

    (bare_url, query)
}

pub async fn request_handler(mut request: HttpRequest) -> Result<()> {
    let start_time = Instant::now();

    match CONFIG.files.get(&request.url) {
        Some(path) => {
            let meta = fs::metadata(&path).await?;
            let content_length = meta.len();

            // Crude mimetypes mappings
            let content_type = lookup_mimetype(&request);

            let file = tokio::fs::OpenOptions::new().read(true).open(&path).await?;
            send_response_header(&mut request, content_type, content_length).await?;
            tokio::io::copy(&mut file.take(content_length), &mut request.stream).await?;
            println!(
                "{} {} ({}) type {}, {} byte(s) in {:?}",
                request.stream.peer_addr()?,
                &request.url,
                path.to_string_lossy(),
                content_type,
                content_length,
                start_time.elapsed()
            );
        }
        None => {
            println!(
                "{} {} not found (404) in {:?}",
                request.stream.peer_addr()?,
                &request.url,
                start_time.elapsed()
            );
            send_response(&mut request, "text/plain", None).await?;
        }
    }

    anyhow::Ok(())
}

pub async fn send_response(
    request: &mut HttpRequest,
    content_type: &str,
    content: Option<&str>,
) -> Result<()> {
    match content {
        Some(content) => Ok(request
            .stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n{}",
                    content_type,
                    &content.len(),
                    &content
                )
                .as_bytes(),
            )
            .await?),
        _ => Ok(request
            .stream
            .write_all(format!("HTTP/1.1 404 Not found\r\n\r\n").as_bytes())
            .await?),
    }
}

pub async fn send_response_header(
    request: &mut HttpRequest,
    content_type: &str,
    content_length: u64,
) -> Result<()> {
    Ok(request
        .stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n",
                content_type, content_length
            )
            .as_bytes(),
        )
        .await?)
}
