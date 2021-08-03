use serde::Serialize;
use anyhow::Result;
use bincode::serialize;
use futures::SinkExt;
use tmq::Context;

#[derive(Serialize, Debug)]
pub struct PVMessage {
    comp: String,
    pkg: String,
    arch: String,
    method: u8,
    from_ver: Option<String>,
    to_ver: Option<String>,
}

impl PVMessage {
    pub fn new(
        comp: String,
        pkg: String,
        arch: String,
        method: u8,
        from_ver: Option<String>,
        to_ver: Option<String>,
    ) -> Self {
        PVMessage {
            comp,
            pkg,
            arch,
            method,
            from_ver,
            to_ver,
        }
    }
}

pub async fn publish_pv_messages(messages: &[PVMessage], ipc_address: &str) -> Result<()> {
    let mut socket = tmq::publish(&Context::new()).bind(ipc_address)?;
    let serialized = serialize(&messages)?;
    socket.send(vec![serialized]).await?;

    Ok(())
}