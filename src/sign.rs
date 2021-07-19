use anyhow::{anyhow, Result};
use openpgp::cert::Cert;
use openpgp::parse::Parse;
use openpgp::policy::StandardPolicy;
use openpgp::serialize::stream::{Armorer, LiteralWriter, Message, Signer};
use sequoia_openpgp as openpgp;
use std::io::Write;
use std::path::Path;

pub fn load_certificate<P: AsRef<Path>>(cert_path: P) -> Result<Cert> {
    Ok(Cert::from_file(cert_path.as_ref())?)
}

pub fn sign_message(cert: &Cert, content: &[u8]) -> Result<Vec<u8>> {
    let policy = StandardPolicy::new();
    let keypair = cert
        .keys()
        .secret()
        .with_policy(&policy, None)
        .supported()
        .alive()
        .revoked(false)
        .for_signing()
        .nth(0);
    if keypair.is_none() {
        return Err(anyhow!("No usable signing key found in your certificate."));
    }
    let keypair = keypair.unwrap().key().clone().into_keypair()?;
    let mut data_sink = Vec::new();
    let message = Message::new(&mut data_sink);
    let message = Armorer::new(message).build()?;
    let message = Signer::new(message, keypair).cleartext().build()?;
    let mut message = LiteralWriter::new(message).build()?;
    message.write_all(content)?;
    message.finalize()?;

    Ok(data_sink)
}
