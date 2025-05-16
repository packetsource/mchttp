#![allow(unused)]

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;
use std::str::FromStr;
use std::fs::Metadata;
use std::borrow::Cow;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::panic;
use std::ffi::OsStr;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::io::BufReader;

use anyhow::{Error, Result};
use lazy_static::lazy_static;
use tokio;
use tokio::net::ToSocketAddrs;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::io::{AsyncBufReadExt, BufStream};
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::task::{spawn, JoinSet, JoinHandle};
use tokio::time::{sleep, Duration, timeout};

use regex::Regex;

use rustls::ConfigBuilder;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs::sign;

use tokio_rustls::rustls::internal::msgs::handshake::ServerExtension::Protocols;
use tokio_rustls::rustls::ProtocolVersion::TLSv1_3;
use tokio_rustls::rustls::SupportedProtocolVersion;
use tokio_rustls;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::TlsAcceptor;

mod config;
use config::*;

mod identity;
use identity::*;

mod http;
use http::*;

mod mimetype;
use mimetype::*;


pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");
pub const COMMIT_ID: &str = env!("GIT_COMMITID");

//#[tokio::main(flavor="current_thread")]
#[tokio::main]
pub async fn main() -> Result<()> {
    dbg!(&PKG_NAME, &PKG_VERSION, &COMMIT_ID);
    dbg!(&*CONFIG);

    let mut tasks = JoinSet::<Result<()>>::new();

    // Start the tasks
    if CONFIG.tls.is_some() {
        let tls_config = CONFIG.tls.as_ref().unwrap();

        // The connection server
        tasks.spawn(async move {
            https_listener(&CONFIG.bind_addr).await
        });

    } else {
        tasks.spawn(async move { http_listener(&CONFIG.bind_addr).await });
    }

    // General task completion handler
    // Print a message indicating success or failure. If it's panic,
    // propagate the error
    while let Some(join_result) = tasks.join_next().await {
        match join_result {
            Ok(result) => {
                eprintln!("Task completed: {:?}", result);
                continue;
            },
            Err(join_error) => {
                if join_error.is_cancelled() {
                    eprintln!("Task cancellation");
                } else if join_error.is_panic() {
                    eprintln!("Task panicked!");
                    if CONFIG.verbose {
                       panic::resume_unwind(join_error.into_panic())
                    }
                } else {
                    eprintln!("Task join error: {:?}", join_error);
                }
            }
        }
    }

    Ok(())
}
