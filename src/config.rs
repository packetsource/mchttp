use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process;

use crate::*;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

#[derive(Debug)]
pub struct Config {
    pub verbose: bool, // -v
    pub bind_addr: SocketAddr,
    pub files: HashMap<String, PathBuf>,
}

lazy_static! {

    // Command line configuration
    pub static ref CONFIG: Config = Config::cmdline();
}

// perhaps make a global structure of above so that it can
// be referred to from anywhere in the namespace without an
// instance variable?

/* Chappell's lightweight getopt() for rust */
impl Default for Config {
    fn default() -> Config {
        Config {
            verbose: false,
            bind_addr: DEFAULT_BIND_ADDR.parse().unwrap(),
            files: HashMap::new(),
        }
    }
}
impl Config {
    pub fn usage() {
        eprintln!("Usage: mchttp [-v] [-l bind_addrs] files");
        eprintln!(
            "       -l            address to bind and listen on ({})",
            &DEFAULT_BIND_ADDR
        );
        eprintln!("       -v            verbose");
        process::exit(1);
    }

    pub fn cmdline() -> Config {
        let mut config = Config::default();

        let mut args = env::args();
        args.next(); // blow off the first argument
        while let Some(a) = args.next() {
            match a.as_str() {
                "-v" => {
                    config.verbose = true;
                    continue;
                }
                "-h" => {
                    Self::usage();
                    break;
                }
                "-?" => {
                    Self::usage();
                    break;
                }

                // Derive the canonical form of the path being specified
                // If it relative to our current directory, load it into the
                // hash with a p
                _ => match std::fs::canonicalize(&a) {
                    Ok(p) => {
                        if a.starts_with("/") {
                            config.files.insert(a, p);
                        } else {
                            config.files.insert(format!("/{}", a), p);
                        }
                    }
                    _ => {
                        eprintln!("Ignoring {}", &a);
                    }
                },
            };
        }

        // Add a make-shift default here
        // if config.files.len() == 0 {
        //     config.files.push(PathBuf::from("/tmp/test.txt"));
        // }
        config
    }
}
