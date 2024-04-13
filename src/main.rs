#![allow(unused_imports)]

use anyhow::{Error, Result};
use lazy_static::lazy_static;
use std::net::SocketAddr;
use std::panic;
use tokio;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::task::{self, JoinError};
use tokio::time::{sleep, Duration};

mod config;
use crate::config::*;

mod http;
use crate::http::*;

pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");
pub const COMMIT_ID: &str = env!("GIT_COMMITID");

//#[tokio::main(flavor="current_thread")]
#[tokio::main]
pub async fn main() -> Result<()> {
    dbg!(&PKG_NAME, &PKG_VERSION, &COMMIT_ID);

    let mut tasks = JoinSet::<Result<()>>::new();

    if CONFIG.verbose {
        dbg!(&*CONFIG);
    }

    for bind_addr in &CONFIG.bind_addr {
        tasks.spawn(async move { http_listener(&bind_addr).await });
    }

    while let Some(join_result) = tasks.join_next().await {
        match join_result {
            Ok(_) => continue,
            Err(join_error) => {
                if join_error.is_cancelled() {
                    eprintln!("Async task cancellation");
                } else if join_error.is_panic() {
                    eprintln!("Async task panicked!");
                    panic::resume_unwind(join_error.into_panic())
                }
            }
        }
    }

    Ok(())
}
