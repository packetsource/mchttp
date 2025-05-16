use crate::*;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

//#[derive(Debug)]
#[derive(Debug)]
pub struct Config {
    pub verbose: bool, // -v
    pub bind_addr: SocketAddr,
    pub files: HashMap<String, PathBuf>,
    pub data_dir: Option<PathBuf>,
    // pub tls: Option<rustls::ServerConfig>,
    // pub tls_cert_filename: Option<String>,
    // pub tls_key_filename: Option<String>,
    pub tls: Option<String>,
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
            data_dir: None,
            tls: None,
            // tls_key_filename: None,
            // tls_cert_filename: None,
            // tls_store: None,
        }
    }
}
impl Config {
    pub fn usage() {
        eprintln!("Usage: mchttp [-v] [-l bind_addr] [-t file/dir] [-r file] files");
        eprintln!("       -v            verbose\n");
        eprintln!("       -l            address to bind and listen on ({})", &DEFAULT_BIND_ADDR);
        eprintln!("       -t file.key   use TLS with file.key and file.crt as default site");
        eprintln!("       -t file.crt   use TLS with file.key and file.crt as default site");
        eprintln!("       -t /etc/letsencrypt/live");
        eprintln!("                     use TLS for all sites specified in LetsEncrypt/Certbot directory");
        eprintln!("                     (ensure readable permissions for UID or GID server runs as)");

        eprintln!("Generate self-signed key/cert like this:");
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
                },
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
                    let file = args.next().expect("expected path to TLS certificate/identity store");
                    std::fs::exists(&file).expect("path to TLS certificate/identity store should exist and be readble");
                    config.tls = Some(file);
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
                "-d" => {
                    config.data_dir = Some(std::fs::canonicalize(
                        args.next().expect("Expected path of data directory")
                    ).expect("Specified data directory path does not exist"));
                    continue;
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
