use anyhow::{Result, anyhow};
use openpgp::cert::{Cert, CertBuilder};
use openpgp::parse::Parse;
use openpgp::policy::StandardPolicy;
use openpgp::serialize::SerializeInto;
use openpgp::serialize::stream::{Message, Signer};
use openpgp::types::KeyFlags;
use sailfish::TemplateSimple;
use secrecy::SecretSlice;
use sequoia_openpgp as openpgp;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

const CERT_LIFETIME: u64 = 2 * 31_556_952; // ~2 years

pub struct GeneratedCert {
    pub id: String,
    pub pubkey: SecretSlice<u8>,
    pub privkey: SecretSlice<u8>,
    pub expiry: u64,
}

#[derive(TemplateSimple)]
#[template(path = "gen-key-instructions.stpl")]
struct InstructionsTemplate {
    pubkey: String,
    privkey: String,
    expdate: String,
    config_file: String,
}

pub fn generate_instructions(
    pubkey: String,
    privkey: String,
    expdate: String,
    config_file: &str,
) -> Result<String> {
    Ok(InstructionsTemplate {
        pubkey,
        privkey,
        expdate,
        config_file: config_file.to_string(),
    }
    .render_once()?)
}

pub fn generate_certificate(userid: &str) -> Result<GeneratedCert> {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs();
    let (cert, _) = CertBuilder::new()
        .add_userid(userid)
        .set_validity_period(Duration::from_secs(CERT_LIFETIME))
        .add_subkey(
            KeyFlags::empty().set_signing().set_authentication(),
            None,
            None,
        )
        .generate()?;
    let pubkey = SecretSlice::from(cert.armored().to_vec()?);
    let privkey = SecretSlice::from(cert.as_tsk().armored().to_vec()?);
    let id = cert.fingerprint().to_string();
    // -60 because sequoia backdates the timestamp by 60 seconds to make signatures immediately binding
    let expiry = now + CERT_LIFETIME - 60;

    Ok(GeneratedCert {
        id,
        pubkey,
        privkey,
        expiry,
    })
}

pub fn load_certificate<P: AsRef<Path>>(cert_path: P) -> Result<Cert> {
    Cert::from_file(cert_path.as_ref())
}

pub fn sign_message_agent(cert: &Cert, content: &[u8]) -> Result<Vec<u8>> {
    use sequoia_gpg_agent::KeyPair;
    use sequoia_gpg_agent::gnupg::Context;

    let policy = StandardPolicy::new();
    let keypair = cert
        .keys()
        .with_policy(&policy, None)
        .supported()
        .alive()
        .revoked(false)
        .for_signing()
        .next();
    if keypair.is_none() {
        return Err(anyhow!("No usable signing key found in your certificate."));
    }
    let pubkey = keypair.unwrap().key();
    let ctx = Context::new()?;
    let offloaded_keypair = KeyPair::new_for_gnupg_context(&ctx, pubkey)?;
    let mut data_sink = Vec::new();
    let message = Message::new(&mut data_sink);
    let mut message = Signer::new(message, offloaded_keypair)?
        .cleartext()
        .build()?;
    message.write_all(content)?;
    message.finalize()?;

    Ok(data_sink)
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
        .next();
    if keypair.is_none() {
        return Err(anyhow!("No usable signing key found in your certificate."));
    }
    let keypair = keypair.unwrap().key().clone().into_keypair()?;
    let mut data_sink = Vec::new();
    let message = Message::new(&mut data_sink);
    let mut message = Signer::new(message, keypair)?.cleartext().build()?;
    message.write_all(content)?;
    message.finalize()?;

    Ok(data_sink)
}
