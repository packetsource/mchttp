use rustls::server;
use crate::*;

#[derive(Debug)]
pub struct HttpRequest<S> {
    pub server_name: Option<String>,
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
        let (mut stream, addr) = listener.accept().await?;

        if CONFIG.verbose {
            eprintln!("HTTP: {:?} connected on FD {}", &addr, &stream.as_raw_fd());
        }
        spawn(async move {
            match process(&mut stream, addr, None).await {
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

    // Prepare TLS configuration
    let tls = CONFIG.tls.as_ref().unwrap();

    // TCP listener for incoming connections
    let listener = TcpListener::bind(addr).await?;

    'listener: loop {
        let mut identity_resolver = server::ResolvesServerCertUsingSni::new();
        load_identities(&mut identity_resolver, tls)?;

        let server_config = Arc::new(
            ServerConfig::builder_with_protocol_versions(
                &[&rustls::version::TLS13, &rustls::version::TLS12]).
                with_no_client_auth().
                with_cert_resolver(Arc::new(identity_resolver))
        );
        let mut tls_acceptor = TlsAcceptor::from(server_config);

        'acceptor: loop {

            let (stream, addr): (TcpStream, SocketAddr) = tokio::select!{

                _ = tokio::time::sleep(Duration::from_secs(60)) => continue 'listener,

                // Received incoming TCP connection
                result = listener.accept() => {
                    result?
                }
            };

            let raw_fd = stream.as_raw_fd();
            if CONFIG.verbose {
                eprintln!("TCP: {:?} connected on FD {}", &addr, &raw_fd);
            }

            let tls_acceptor = tls_acceptor.clone();

            spawn(async move {

                let mut stream = match tls_acceptor.accept(stream).await {
                    Ok(stream) => stream,
                    Err(e) => {
                        eprintln!("HTTPS: {:?} connected on FD {}: error: {:?}", &addr, &raw_fd, &e);
                        return;
                    }
                };
                let (_, server_conn) = stream.get_ref();
                let server_name = server_conn.server_name().map(|x| x.to_string());
                if CONFIG.verbose {
                    eprintln!("HTTPS: {:?} connected on FD {} (identity {:?}, cipher suite {:?})",
                              &addr, &stream.as_raw_fd(),
                              &server_conn.server_name(),
                              &server_conn.negotiated_cipher_suite());
                }

                match timeout(Duration::from_secs(5), process(&mut stream, addr, server_name)).await {
                    Err(e) => {
                        eprintln!("HTTPS: {:?}: connection handler timed out", &addr);
                    },
                    Ok(result) => match result {
                        Ok(()) => {},
                        Err(e) => {
                            eprintln!("Error encountered while handling request for client {}: {e}", &addr);
                        }
                    }
                }

                // Close out TLS nicely
                let (_, mut server_conn) = stream.get_mut();
                server_conn.send_close_notify();
                stream.flush().await;

                if CONFIG.verbose {
                    eprintln!("HTTPS: {:?} closed", &addr);
                }
            });
        }
    }
}


// Entry point per HTTP client connection.
// Read HTTP request and create HTTPRequest structure appropriately
// Then call request_handler() to service the request
pub async fn process<S: AsyncRead + AsyncWrite + std::marker::Unpin>(stream: &mut S,
    client: SocketAddr, server_name: Option<String>) -> Result<()> {
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
            return Err(Error::msg(format!("HTTP: {}: client EOF", &client)));
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
                println!("Process: server {}: {}: request: {}", &server_name.as_ref().map_or("default", |x| x), &client, &line);
                let verb_tokens: Vec<&str> = line.split(" ").collect();
                if verb_tokens.len() != 3 {
                    return Err(Error::msg(
                        format!("Process: {}: malformed HTTP request (method/request/version)", &client
                    )));
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
        server_name,
        client,
        method,
        url,
        version,
        stream,
        headers,
        query,
    };

    request_handler_dir(http_request).await
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

// Static file based request handler
pub async fn request_handler_static_file<S: AsyncRead + AsyncWrite + Unpin>(mut request: HttpRequest<S>) -> Result<()> {
    let start_time = Instant::now();

    match CONFIG.files.get(&request.url) {
        Some(path) => {
            let meta = tokio::fs::metadata(&path).await?;
            let content_length = meta.len();

            // Crude mimetypes mappings
            let content_type = lookup_mimetype(&path);

            let file = tokio::fs::OpenOptions::new().read(true).open(&path).await?;
            send_response_header(&mut request, content_type, content_length).await?;
            tokio::io::copy(&mut file.take(content_length), &mut request.stream).await?;
            // request.stream.flush().await?;

            println!(
                "Request {} {} ({}) type {}, {} byte(s) in {:?}",
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
                "Request {} {} not found (404) in {:?}",
                &request.client,
                &request.url,
                start_time.elapsed()
            );
            send_response(&mut request, "text/plain", None).await?;
        }
    }

    request.stream.flush().await?;
    anyhow::Ok(())
}

pub async fn request_handler_dir<S: AsyncRead + AsyncWrite + Unpin>(mut request: HttpRequest<S>) -> Result<()> {

    let start_time = Instant::now();

    // Build the content root directory structure
    let mut root_path: PathBuf = PathBuf::new();
    if let Some(data_dir) = &CONFIG.data_dir {
        root_path.push(data_dir);
    }
    if let Some(server_name) = &request.server_name {
        root_path.push(server_name);
    }

    // Create a new path structure for the requested URL, anchored in the root
    let mut request_path = PathBuf::from(&root_path);

    // Disregard leading /s in the request.
    let url_path = PathBuf::from(&request.url);

    match url_path.strip_prefix("/") {
        Err(e) => {
            request_path.push(url_path);
        },
        Ok(url_path) => {
            request_path.push(url_path);
        }
    }

    // Append index.html if necessary
    if request_path.is_dir() {
        request_path.push("index.html");
    }

    // Calculate the canonicalised version of the path
    match std::fs::canonicalize(&request_path) {
        Ok(path) => {

            // Does the request path start with the root path?
            if path.starts_with(root_path) {

                let content_type = lookup_mimetype(&path);
                // eprintln!("Looking at path {}", &path.to_string_lossy());
                // eprintln!("Looking at path normalised as {}", std::fs::canonicalize(&path)?.to_string_lossy());

                match tokio::fs::metadata(&path).await {
                    Ok(meta) => {
                        let content_length = meta.len();
                        let mut file = tokio::fs::OpenOptions::new().read(true).open(&path).await?;
                        send_response_header(&mut request, content_type, content_length).await?;
                        tokio::io::copy(&mut file.take(content_length), &mut request.stream).await?;
                        println!(
                            "Request (server {}) client {} {} {} ({}) type {}, {} byte(s) in {:?}",
                            &request.server_name.as_ref().map_or("default", |x| x),
                            &request.client,
                            &request.method,
                            &request.url,
                            path.to_string_lossy(),
                            content_type,
                            content_length,
                            start_time.elapsed()
                        );
                    },
                    Err(e) => {
                        println!(
                            "Request (server {}) {} {} {} not found (metadata) (404) in {:?}",
                                &request.server_name.as_ref().map_or("default", |x| x),
                                &request.client,
                                &request.method,
                                &request.url,
                                start_time.elapsed()
                        );
                        send_response(&mut request, "text/plain", None).await;
                    } 
                }
            } else {
                eprintln!("Request (server {}) client {} {} {}: illegal access request: {}",
                    &request.server_name.as_ref().map_or("default", |x| x),
                        &request.client,
                        &request.method,
                        &request.url,
                        &request_path.to_string_lossy());
                send_response(&mut request, "text/plain", None).await;
            }
        },
        _ => {
            println!(
                "Request (server {}) {} {} {} not found (canonical) (404) in {:?}",
                    &request.server_name.as_ref().map_or("default", |x| x),
                    &request.client,
                    &request.method,
                    &request.url,
                    start_time.elapsed()
            );
            send_response(&mut request, "text/plain", None).await;
        }
    };

    request.stream.flush().await?;
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
   Ok(())
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
