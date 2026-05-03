# Simplistic HTTP server in Rust, making use of Tokio async I/O framework

**mchttp** is a minimal async HTTP/HTTPS static file server written in 
Rust using Tokio. It serves individual mapped files or directory trees, 
with optional TLS/SNI support via rustls.

Usage:
```
mchttp [-v] [-l 0.0.0.0:8080] [-t <tls-cert-dir-or-file>] [-r <root-dir>] [-d <data-dir>] [file...]
  -v         verbose logging
  -l <addr>  bind address (default: 0.0.0.0:8080)
  -t <path>  TLS: path to cert/key files or a LetsEncrypt/Certbot directory
  -r <path>  serve this directory at /
  -d <path>  data directory
  file...    map individual files to /<filename> routes
```

No warranty
