use rustls::server;
use crate::*;

#[derive(Debug)]
pub struct CertResolver {
    pub sni: server::ResolvesServerCertUsingSni,
    pub default: Option<Arc<rustls::sign::CertifiedKey>>,
}

impl rustls::server::ResolvesServerCert for CertResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
        self.sni.resolve(hello).or_else(|| self.default.clone())
    }
}

pub fn load_identities(identity_resolver: &mut CertResolver, path: &str) -> Result<()> {

    let mut files: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    let metadata = std::fs::metadata(path)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let dns_name = match path.file_name() {
                    Some(s) => match s.to_str() {
                        Some(s) => String::from(s),
                        None => continue,
                    }
                    None => continue,
                };

                let mut cert_path: std::path::PathBuf = entry.path();
                cert_path.push("fullchain.pem");

                let mut key_path: std::path::PathBuf = entry.path();
                key_path.push("privkey.pem");

                dbg!((&dns_name, &cert_path, &key_path));
                files.push((dns_name, cert_path, key_path));
            }
        }

    } else if metadata.is_file() {
        let dns_name = match PathBuf::from(&path).file_stem() {
            Some(s) => match s.to_str() {
                Some(s) => String::from(s),
                None => return Err(anyhow::anyhow!("Path name supplied for identity/certificate should be in format my.domain.name.crt/key"))
            }
            None => return Err(anyhow::anyhow!("Path name supplied for identity/certificate should be in format my.domain.name.crt/key"))
        };
        let (key_path, cert_path): (PathBuf, PathBuf) = {
            if path.ends_with(".key") {
                (PathBuf::from(&path), PathBuf::from(path.replace(".key", ".crt")))
            } else if path.ends_with(".crt") {
                (PathBuf::from(path.replace(".crt", ".key")), PathBuf::from(&path))
            } else {
                return Err(anyhow::anyhow!("Identity/certificate filename should have either .key or .crt suffix, respectively"));
            }
        };
        files.push((dns_name, cert_path, key_path));
    }

    let mut count: u64 = 0;
    for (dns_name, cert_path, key_path) in &files {
        let certs= CertificateDer::pem_file_iter(&cert_path)?
            .collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(&key_path)?;
        let certified_key = rustls::sign::CertifiedKey::new(
            certs.clone(),
            sign::any_supported_type(&key).unwrap(),
        );
        // We put the first entry in as the default
        if count == 0 {
            identity_resolver.default = Some(Arc::new(certified_key.clone()));
        }
        let _ = identity_resolver.sni.add(dns_name, certified_key)?;
        count += 1;
    }

    Ok(())
}

