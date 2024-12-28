use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio_rustls::TlsAcceptor;
use crate::*;

#[derive(Debug)]
pub struct HttpRequest<S> {
    pub client: SocketAddr,
    pub method: String,
    pub url: String,
    pub version: String,
    pub stream: BufStream<S>,
    pub headers: HashMap<String, String>,
    pub query: HashMap<String, String>,
}

// Main HTTP listener. Binds to port and spawns a new task calling process()
// for each incoming request
pub async fn http_listener<A: ToSocketAddrs + ?Sized>(addr: &A) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, addr) = listener.accept().await?;

        if CONFIG.verbose {
            eprintln!("HTTP: {:?} connected on FD {}", &addr, &stream.as_raw_fd());
        }
        spawn(async move {
            match process(stream, addr).await {
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

pub async fn https_listener<A: ToSocketAddrs + ?Sized>(addr: &A) -> Result<()> {

    let listener = TcpListener::bind(addr).await?;

    'listener: loop {
        let original_cert_mtime = std::fs::metadata(
            CONFIG.tls_cert_filename.as_ref().unwrap()
        )?.modified()?;

        let tls_acceptor = TlsAcceptor::from(Arc::new(CONFIG.tls.as_ref().unwrap().clone()));

        'acceptor: loop {
            let current_cert_mtime = std::fs::metadata(
                CONFIG.tls_cert_filename.as_ref().unwrap()
            )?.modified()?;
            if current_cert_mtime > original_cert_mtime {
                eprintln!("HTTPS: certificate file has changed: restarting TLS acceptor");
                break 'acceptor;
            }

            let (stream, addr) = listener.accept().await?;
            if CONFIG.verbose {
                eprintln!("TCP: {:?} connected on FD {}", &addr, &stream.as_raw_fd());
            }

            let tls_acceptor = tls_acceptor.clone();
            let stream = match tls_acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(e) => {
                    eprintln!("HTTPS: {:?}: connection error: {:?}", &addr, &e);
                    continue;
                }
            };
            if CONFIG.verbose {
                eprintln!("HTTPS: {:?} connected on FD {}", &addr, &stream.as_raw_fd());
            }

            spawn(async move {
                match process(stream, addr).await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Error encountered while handling request: {e}");
                    }
                }
                if CONFIG.verbose {
                    eprintln!("HTTPS: {:?} closed", &addr);
                }
                anyhow::Ok(())
            });
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}


// Entry point per HTTP client connection.
// Read HTTP request and create HTTPRequest structure appropriately
// Then call request_handler() to service the request
pub async fn process<S: AsyncRead + AsyncWrite + std::marker::Unpin>(stream: S, client: SocketAddr) -> Result<()> {
    //let mut reader = BufReader::new(stream);
    let mut stream = tokio::io::BufStream::new(stream);
    let mut line_count: u64 = 0;

    let mut method = String::new();
    let mut url = String::new();
    let mut version = String::new();
    let mut headers = HashMap::<String, String>::new();
    let mut query = HashMap::<String, String>::new();

    // Read the header
    loop {
        let mut buf = Vec::<u8>::new();
        let bytes_read = stream.read_until('\n' as u8, &mut buf).await?;

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
                println!("Request: {}: {}", &client, &line);
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
        client,
        method,
        url,
        version,
        stream,
        headers,
        query,
    };

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

pub async fn request_handler<S: AsyncRead + AsyncWrite + Unpin>(mut request: HttpRequest<S>) -> Result<()> {
    let start_time = Instant::now();

    match CONFIG.files.get(&request.url) {
        Some(path) => {
            let meta = fs::metadata(&path).await?;
            let content_length = meta.len();

            // Crude mimetypes mappings
            let content_type = lookup_mimetype(&path);

            let file = tokio::fs::OpenOptions::new().read(true).open(&path).await?;
            send_response_header(&mut request, content_type, content_length).await?;
            tokio::io::copy(&mut file.take(content_length), &mut request.stream).await?;

            println!(
                "Response: {} {} ({}) type {}, {} byte(s) in {:?}",
                &request.client,
                &request.url,
                path.to_string_lossy(),
                content_type,
                content_length,
                start_time.elapsed()
            );
        },
        None => {
            println!(
                "{} {} not found (404) in {:?}",
                &request.client,
                &request.url,
                start_time.elapsed()
            );
            send_response(&mut request, "text/plain", None).await?;
        }
    }

    anyhow::Ok(())
}

pub async fn send_response<S>(
    request: &mut HttpRequest<S>,
    content_type: &str,
    content: Option<&str>,
) -> anyhow::Result<()> where
    BufStream<S>: AsyncWrite + AsyncRead, S: AsyncWrite + AsyncRead + Unpin
{
    match content {
        Some(content) => {
            request
                .stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n{}",
                            content_type,
                            &content.len(),
                            &content
                        ).as_bytes(),
                    )
                .await?;
        },
        _ => {
            request
                .stream
                .write_all(String::from("HTTP/1.1 404 Not found\r\n\r\n").as_bytes())
                .await?;
        },
    };
   Ok(request.stream.flush().await?)
}

pub async fn send_response_header<S> (
    request: &mut HttpRequest<S>,
    content_type: &str,
    content_length: u64,
) -> Result<()> where
    BufStream<S>: AsyncWrite + AsyncRead, S: AsyncWrite + AsyncRead + Unpin
{
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
