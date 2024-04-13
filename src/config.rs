use std::env;
use std::process;

use crate::*;

#[derive(Debug)]
pub struct Config {
    pub verbose: bool, // -v
    pub bind_addr: Vec<String>,
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
            bind_addr: vec![],
        }
    }
}
impl Config {
    pub fn usage() {
        eprintln!("Usage: mchttp [-v] bind_addrs");
        eprintln!("       -v            verbose");
        process::exit(1);
    }

    pub fn cmdline() -> Config {
        let mut config = Config::default();

        let mut args = env::args();
        args.next(); // blow off the first argument
        while let Some(a) = args.next() {
            config.bind_addr.push(match a.as_str() {
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
                _ => a,
            })
        }

        // Add a make-shift default here
        if config.bind_addr.len() == 0 {
            config.bind_addr.push(String::from("0.0.0.0:8002"));
        }
        config
    }
}
