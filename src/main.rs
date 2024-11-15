#![allow(unused_imports)]

use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;
use std::str::FromStr;
use std::borrow::Cow;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use std::panic;

use anyhow::{Error, Result};
use lazy_static::lazy_static;
use tokio;
use tokio::fs;
use tokio::net::ToSocketAddrs;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::io::{AsyncBufReadExt, BufStream};
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::task::{spawn, JoinSet, JoinHandle};
use tokio::time::{sleep, Duration};

use regex::Regex;

use native_tls::{Identity};
use tokio_native_tls::{TlsAcceptor, TlsStream};

mod config;
use crate::config::*;

mod http;
use crate::http::*;

mod mimetype;
use crate::mimetype::*;

pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");
pub const COMMIT_ID: &str = env!("GIT_COMMITID");

#[derive(Clone)]
pub struct TlsIdentity(Option<Identity>);

//#[tokio::main(flavor="current_thread")]
#[tokio::main]
pub async fn main() -> Result<()> {
    dbg!(&PKG_NAME, &PKG_VERSION, &COMMIT_ID);
    dbg!(&*CONFIG);

    let mut tasks = JoinSet::<Result<()>>::new();

    tasks.spawn(async move { listener(&CONFIG.bind_addr).await });

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
