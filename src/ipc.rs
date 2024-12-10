use anyhow::Result;
use redis::{Commands, Connection};
use serde::Serialize;

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

pub fn redis_connect(ipc_address: &str) -> Result<Connection> {
    let client = redis::Client::open(ipc_address)?;
    let con = client.get_connection()?;

    Ok(con)
}

pub fn publish_pv_messages(messages: &[PVMessage], conn: &mut Connection) -> Result<()> {
    let serialized = serde_json::to_string(&messages)?;
    conn.publish::<_, _, ()>("p-vector-publish", serialized)?;

    Ok(())
}
