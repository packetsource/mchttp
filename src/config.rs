use std::fmt::Formatter;
use crate::*;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

impl std::fmt::Debug for TlsIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.0.is_none() {
            write!(f, "none")
        } else {
            write!(f, "identity-provided")
        }
    }
}

//#[derive(Debug)]
#[derive(Debug)]
pub struct Config {
    pub verbose: bool, // -v
    pub bind_addr: SocketAddr,
    pub files: HashMap<String, PathBuf>,
    pub tls: TlsIdentity,
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
            tls: TlsIdentity(None),
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
        eprintln!("       -t            identity.p12 filename for HTTPS");
        eprintln!("       -v            verbose\n");

        eprintln!("Generate key/cert like this:");
        eprintln!(" /usr/bin/openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes");
        //eprintln!(" /usr/bin/openssl pkcs12 -export -out cert.p12 -inkey key.pem -in cert.pem");

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
                "-l" => {
                    config.bind_addr = SocketAddr::from_str(
                        args.next()
                            .expect("expected bind address specification")
                            .as_str(),
                    )
                    .expect("failued to parse bind address specification");
                    continue;
                },
                "-t" => {

                    let (identity, cert) = {
                        let root = args.next()
                            .expect("expected filename.key / filename.crt containing TLS key or certificate");
                        if root.contains(".key") {
                            (root.clone(), root.replace(".key", ".crt"))
                        } else if root.contains(".crt") {
                            (root.replace(".crt", ".key"), root.clone())
                        } else {
                            eprintln!("-t requires filename.key or filename.crt");
                            Self::usage();
                            break;
                        }
                    };
                    config.tls = TlsIdentity (
                        Some(Identity::from_pkcs8(&std::fs::read(cert).expect("couldn't read public certificate file"),
                                                  &std::fs::read(identity).expect("couldn't read identity/key file"))
                            .expect("couldn't process TLS identity"))
                    );
                    continue;
                },
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
