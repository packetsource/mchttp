use rustls::server;
use std::pin::Pin;
use std::task::{Context, Poll};
use crate::*;

const MAX_HEADER_LINES: usize = 100;
const MAX_LINE_BYTES: usize = 8 * 1024;

// Unified stream type so a single listener handles both HTTP and HTTPS.
// Both TcpStream and TlsStream<TcpStream> implement AsyncRead + AsyncWrite + Unpin,
// so AnyStream inherits all of those automatically.
pub enum AnyStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

impl tokio::io::AsyncRead for AnyStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            AnyStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for AnyStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            AnyStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            AnyStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyStream::Plain(s) => Pin::new(s).poll_flush(cx),
            AnyStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AnyStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            AnyStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

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

// Build a TlsAcceptor from CONFIG, returning None if TLS is not configured or
// identity loading fails.
fn build_tls_acceptor() -> Option<TlsAcceptor> {
    let tls = CONFIG.tls.as_ref()?;
    let mut identity_resolver = server::ResolvesServerCertUsingSni::new();
    if let Err(e) = load_identities(&mut identity_resolver, tls) {
        eprintln!("Failed to load TLS identities: {e}");
        return None;
    }
    let config = ServerConfig::builder_with_protocol_versions(
        &[&rustls::version::TLS13, &rustls::version::TLS12],
    )
    .with_no_client_auth()
    .with_cert_resolver(Arc::new(identity_resolver));
    Some(TlsAcceptor::from(Arc::new(config)))
}

// Single listener for both HTTP and HTTPS. TLS presence is determined by CONFIG.
// A background task reloads certificates every 60 seconds when TLS is active.
pub async fn listener<A: ToSocketAddrs + ?Sized>(addr: &A) -> Result<()> {
    let tcp = TcpListener::bind(addr).await?;
    let shared = Arc::new(Mutex::new(build_tls_acceptor()));

    if CONFIG.tls.is_some() {
        let shared = Arc::clone(&shared);
        spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                *shared.lock().unwrap() = build_tls_acceptor();
            }
        });
    }

    loop {
        let (stream, addr) = tcp.accept().await?;
        let acceptor = shared.lock().unwrap().clone();
        let is_tls = acceptor.is_some();
        let raw_fd = stream.as_raw_fd();

        if CONFIG.verbose {
            eprintln!(
                "{}: {:?} connected on FD {}",
                if is_tls { "HTTPS" } else { "HTTP" },
                &addr,
                raw_fd
            );
        }

        spawn(async move {
            let result: Result<()> = match acceptor {
                None => {
                    let mut s = AnyStream::Plain(stream);
                    process(&mut s, addr, None).await
                }
                Some(acceptor) => match acceptor.accept(stream).await {
                    Err(e) => {
                        eprintln!(
                            "HTTPS: {:?} FD {}: TLS handshake error: {:?}",
                            &addr, raw_fd, e
                        );
                        return;
                    }
                    Ok(tls_stream) => {
                        let server_name = {
                            let (_, conn) = tls_stream.get_ref();
                            conn.server_name().map(str::to_string)
                        };
                        if CONFIG.verbose {
                            let (_, conn) = tls_stream.get_ref();
                            eprintln!(
                                "HTTPS: {:?} FD {} identity {:?} cipher {:?}",
                                &addr,
                                raw_fd,
                                conn.server_name(),
                                conn.negotiated_cipher_suite()
                            );
                        }
                        let mut s = AnyStream::Tls(Box::new(tls_stream));
                        let r = match timeout(
                            Duration::from_secs(5),
                            process(&mut s, addr, server_name),
                        )
                        .await
                        {
                            Err(_) => {
                                eprintln!("HTTPS: {:?}: timed out", &addr);
                                Ok(())
                            }
                            Ok(r) => r,
                        };
                        if let AnyStream::Tls(ref mut tls) = s {
                            // send_close_notify borrow ends at ;
                            tls.get_mut().1.send_close_notify();
                            let _ = tls.flush().await;
                        }
                        r
                    }
                },
            };

            if let Err(e) = result {
                eprintln!(
                    "{}: {:?}: error: {e}",
                    if is_tls { "HTTPS" } else { "HTTP" },
                    &addr
                );
            }
            if CONFIG.verbose {
                eprintln!(
                    "{}: {:?} closed",
                    if is_tls { "HTTPS" } else { "HTTP" },
                    &addr
                );
            }
        });
    }

    #[allow(unreachable_code)]
    Ok(())
}

// Entry point per HTTP client connection — parse HTTP request then dispatch.
pub async fn process<S: AsyncRead + AsyncWrite + std::marker::Unpin>(
    stream: &mut S,
    client: SocketAddr,
    server_name: Option<String>,
) -> Result<()> {
    let mut stream = tokio::io::BufStream::new(stream);
    let mut line_count: usize = 0;

    let mut method = String::new();
    let mut url = String::new();
    let mut version = String::new();
    let mut headers = HashMap::<String, String>::new();
    let mut query = HashMap::<String, String>::new();

    loop {
        let mut buf = Vec::<u8>::new();
        let bytes_read = stream.read_until(b'\n', &mut buf).await?;

        if bytes_read == 0 {
            return Err(Error::msg(format!("HTTP: {}: client EOF", &client)));
        }

        // Reject oversized lines before allocating a String from them.
        if bytes_read > MAX_LINE_BYTES {
            stream
                .write_all(b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await?;
            stream.flush().await?;
            return Err(Error::msg(format!(
                "HTTP: {}: header line too large ({} bytes)",
                &client, bytes_read
            )));
        }

        if bytes_read > 2 {
            let mut line = String::from_utf8(buf)?;

            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }

            if line_count == 0 {
                println!(
                    "Process: server {}: {}: request: {}",
                    &server_name.as_ref().map_or("default", |x| x),
                    &client,
                    &line
                );
                let verb_tokens: Vec<&str> = line.split(' ').collect();
                if verb_tokens.len() != 3 {
                    return Err(Error::msg(format!(
                        "Process: {}: malformed HTTP request (method/request/version)",
                        &client
                    )));
                }
                method = verb_tokens[0].to_lowercase();
                (url, query) = query_string(verb_tokens[1]);
                version = verb_tokens[2].to_string();
            } else if let Some((k, v)) = line
                .split_once(':')
                .map(|x| (x.0.to_lowercase(), x.1.trim().to_string()))
            {
                headers.insert(k, v);
            }

            line_count += 1;
            if line_count > MAX_HEADER_LINES {
                stream
                    .write_all(b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await?;
                stream.flush().await?;
                return Err(Error::msg(format!(
                    "HTTP: {}: too many header lines",
                    &client
                )));
            }
        } else {
            break;
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

// Split URL on '?' first (before decoding) to prevent %3F from being misread
// as a query delimiter. Decode path and query values independently.
pub fn query_string(url: &str) -> (String, HashMap<String, String>) {
    let mut query = HashMap::<String, String>::new();

    match url.split_once('?') {
        None => {
            let bare = urlencoding::decode(url)
                .unwrap_or_else(|_| url.into())
                .into_owned();
            (bare, query)
        }
        Some((path, qs)) => {
            let bare = urlencoding::decode(path)
                .unwrap_or_else(|_| path.into())
                .into_owned();
            for kv in qs.split('&') {
                if let Some((k, v)) = kv.split_once('=') {
                    let v = urlencoding::decode(v)
                        .unwrap_or_else(|_| v.into())
                        .into_owned();
                    query.insert(k.to_lowercase(), v);
                }
            }
            (bare, query)
        }
    }
}

pub async fn request_handler_static_file<S: AsyncRead + AsyncWrite + Unpin>(
    mut request: HttpRequest<S>,
) -> Result<()> {
    let start_time = Instant::now();

    match CONFIG.files.get(&request.url) {
        Some(path) => {
            let meta = tokio::fs::metadata(&path).await?;
            let content_length = meta.len();
            let content_type = lookup_mimetype(&path);
            let file = tokio::fs::OpenOptions::new().read(true).open(&path).await?;
            send_response_header(&mut request, content_type, content_length).await?;
            tokio::io::copy(&mut file.take(content_length), &mut request.stream).await?;
            println!(
                "Request {} {} ({}) type {}, {} byte(s) in {:?}",
                &request.client,
                &request.url,
                path.to_string_lossy(),
                content_type,
                content_length,
                start_time.elapsed()
            );
        }
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

pub async fn request_handler_dir<S: AsyncRead + AsyncWrite + Unpin>(
    mut request: HttpRequest<S>,
) -> Result<()> {
    let start_time = Instant::now();

    // Build root path from configuration
    let mut root_path = PathBuf::new();
    if let Some(data_dir) = &CONFIG.data_dir {
        root_path.push(data_dir);
    }
    if let Some(server_name) = &request.server_name {
        root_path.push(server_name);
    }

    // Canonicalize root_path now so symlinks in data_dir/server_name can't
    // bypass the starts_with check below.
    let root_path_base = if root_path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        root_path.clone()
    };
    let canon_root = match tokio::fs::canonicalize(&root_path_base).await {
        Ok(p) => p,
        Err(_) => {
            send_response(&mut request, "text/plain", None).await?;
            request.stream.flush().await?;
            return Ok(());
        }
    };

    // Anchor the request URL under root_path
    let mut request_path = PathBuf::from(&root_path);
    let url_path = PathBuf::from(&request.url);
    match url_path.strip_prefix("/") {
        Err(_) => request_path.push(url_path),
        Ok(p) => request_path.push(p),
    }

    // Append index.html for directory requests
    if tokio::fs::metadata(&request_path)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        request_path.push("index.html");
    }

    match tokio::fs::canonicalize(&request_path).await {
        Ok(path) => {
            if path.starts_with(&canon_root) {
                let content_type = lookup_mimetype(&path);
                match tokio::fs::metadata(&path).await {
                    Ok(meta) => {
                        let content_length = meta.len();
                        let mut file =
                            tokio::fs::OpenOptions::new().read(true).open(&path).await?;
                        send_response_header(&mut request, content_type, content_length).await?;
                        tokio::io::copy(
                            &mut file.take(content_length),
                            &mut request.stream,
                        )
                        .await?;
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
                    }
                    Err(_) => {
                        println!(
                            "Request (server {}) {} {} {} not found (metadata) (404) in {:?}",
                            &request.server_name.as_ref().map_or("default", |x| x),
                            &request.client,
                            &request.method,
                            &request.url,
                            start_time.elapsed()
                        );
                        send_response(&mut request, "text/plain", None).await?;
                    }
                }
            } else {
                eprintln!(
                    "Request (server {}) client {} {} {}: illegal access request: {}",
                    &request.server_name.as_ref().map_or("default", |x| x),
                    &request.client,
                    &request.method,
                    &request.url,
                    &request_path.to_string_lossy()
                );
                send_response(&mut request, "text/plain", None).await?;
            }
        }
        Err(_) => {
            println!(
                "Request (server {}) {} {} {} not found (canonical) (404) in {:?}",
                &request.server_name.as_ref().map_or("default", |x| x),
                &request.client,
                &request.method,
                &request.url,
                start_time.elapsed()
            );
            send_response(&mut request, "text/plain", None).await?;
        }
    }

    request.stream.flush().await?;
    anyhow::Ok(())
}

pub async fn send_response<S>(
    request: &mut HttpRequest<S>,
    content_type: &str,
    content: Option<&str>,
) -> anyhow::Result<()>
where
    BufStream<S>: AsyncWrite + AsyncRead,
    S: AsyncWrite + AsyncRead + Unpin,
{
    match content {
        Some(content) => {
            request
                .stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        content_type,
                        content.len(),
                        content
                    )
                    .as_bytes(),
                )
                .await?;
        }
        None => {
            request
                .stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await?;
        }
    }
    Ok(())
}

pub async fn send_response_header<S>(
    request: &mut HttpRequest<S>,
    content_type: &str,
    content_length: u64,
) -> Result<()>
where
    BufStream<S>: AsyncWrite + AsyncRead,
    S: AsyncWrite + AsyncRead + Unpin,
{
    Ok(request
        .stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                content_type, content_length
            )
            .as_bytes(),
        )
        .await?)
}
