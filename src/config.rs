use crate::*;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

//#[derive(Debug)]
#[derive(Debug)]
pub struct Config {
    pub verbose: bool, // -v
    pub bind_addr: SocketAddr,
    pub files: HashMap<String, PathBuf>,
    pub tls: Option<rustls::ServerConfig>,
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
            tls: None,
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
        eprintln!("       -t            identity.key/identity.crt/filename for HTTPS");
        eprintln!("       -v            verbose\n");

        eprintln!("Generate key/cert like this:");
        eprintln!("/usr/bin/openssl req -x509 -newkey rsa:4096 -keyout mykey.key -out mycert.crt -days 30 -nodes -addext \"subjectAltName = DNS:localhost\"");
        // eprintln!(" /usr/bin/openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes");
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
                    let certs = CertificateDer::pem_file_iter(&cert)
                        .expect("couldn't load PEM file certificates")
                        .collect::<Result<Vec<_>, _>>()
                        .expect("couldn't read PEM file certificates");
                    let key = PrivateKeyDer::from_pem_file(&identity)
                        .expect("couldn't load private key");
                    config.tls =
                        Some(rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                        .with_no_client_auth()
                        .with_single_cert(certs, key).expect("Couldn't use TLS certificate")
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
                },
                "-r" => {
                    match std::fs::canonicalize(args.next().expect("expect root handler")) {
                        Ok(p) => {
                            config.files.insert(String::from("/"), p);
                        },
                        _ => {
                            eprintln!("Specified root URL doesn't exist");
                        }
                    }
                    continue;
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
