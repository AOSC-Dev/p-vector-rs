use anyhow::Result;
use bincode::serialize;
use serde::Serialize;
use zmq::{Context, Socket, PUB};

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

pub fn zmq_bind(ipc_address: &str) -> Result<Socket> {
    let socket = Context::new().socket(PUB)?;
    socket.bind(ipc_address)?;

    Ok(socket)
}

pub fn publish_pv_messages(messages: &[PVMessage], socket: &Socket) -> Result<()> {
    let serialized = serialize(&messages)?;
    socket.send(serialized, 0)?;

    Ok(())
}
