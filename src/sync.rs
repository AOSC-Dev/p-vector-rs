//! Cross-site database synchronization module

use std::path::Path;

use anyhow::Result;
use async_compression::tokio::write::GzipDecoder;
use log::info;
use reqwest::Client;
use sqlx::{Executor, PgPool};
use tempfile::Builder;
use tokio::{fs::File, io::AsyncWriteExt, task::spawn_blocking};

use crate::db::load_fdw_ext;

const UPSTREAM_URL: &str = "https://packages.aosc.io/data/";
const SYNC_DATABASES: &[&str] = &["abbs.db"];
const PV_SYNC_SQL_SCRIPT: &str = include_str!("../sql/pvsync.sql");

async fn download_db(file: &mut File, component: &str, etag: &str) -> Result<Vec<u8>> {
    let url = format!("{}{}", UPSTREAM_URL, component);
    let client = Client::new();
    let mut resp = client.get(url).header("If-None-Match", etag).send().await?;
    resp.error_for_status_ref()?;
    let new_etag = resp.headers().get("ETag");
    let new_etag = new_etag.map(|x| x.as_bytes()).unwrap_or_default().to_vec();
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(new_etag);
    }
    let mut f = GzipDecoder::new(file);
    // async copy response body to file
    while let Some(chunk) = resp.chunk().await? {
        f.write_all(&chunk).await?;
    }
    // flush cache
    f.shutdown().await?;

    Ok(new_etag)
}

async fn sync_db(pool: &PgPool, db_path: &Path) -> Result<()> {
    sqlx::query!("DROP SERVER IF EXISTS sqlite_server CASCADE")
        .execute(pool)
        .await?;
    // TODO: use parameter binding
    let stmt = format!(
        "CREATE SERVER sqlite_server FOREIGN DATA WRAPPER sqlite_fdw OPTIONS (database '{}')",
        db_path.to_string_lossy()
    );
    pool.execute(&*stmt).await?;
    pool.execute(PV_SYNC_SQL_SCRIPT).await?;

    Ok(())
}

#[cfg(unix)]
fn change_permissions(f: &mut File) -> Result<()> {
    use nix::sys::stat::fchmod;
    use nix::sys::stat::Mode;
    use std::os::unix::prelude::AsRawFd;

    let fd = f.as_raw_fd();
    fchmod(fd, Mode::from_bits_truncate(0o666))?;

    Ok(())
}

pub async fn sync_db_updates(pool: &PgPool) -> Result<()> {
    info!("Synchronizing databases ...");
    load_fdw_ext(pool).await?;
    for db in SYNC_DATABASES {
        info!("Sync {} ...", db);
        let cached = sqlx::query!("SELECT name, etag FROM pv_dbsync WHERE name = $1", db)
            .fetch_optional(pool)
            .await?;
        let etag: String = if let Some(cached) = cached {
            cached.etag.unwrap_or_default()
        } else {
            String::new()
        };
        let temp = spawn_blocking(|| {
            Builder::new()
                .suffix(".db")
                .prefix("pvsync-")
                .tempfile_in("/dev/shm/")
        })
        .await??;
        let temp_path = temp.into_temp_path();
        let mut temp_file = File::create(&temp_path).await?;
        let new_etag = download_db(&mut temp_file, &format!("{}.gz", db), &etag).await?;
        let new_etag = std::str::from_utf8(&new_etag)?;
        if new_etag == etag {
            info!("{} update to date.", db);
            continue;
        }
        #[cfg(unix)]
        {
            spawn_blocking(move || change_permissions(&mut temp_file)).await??;
        }
        sync_db(pool, &temp_path).await?;
        sqlx::query!(
            "INSERT INTO pv_dbsync VALUES ($1, $2, now()) ON CONFLICT (name) DO UPDATE SET etag=$2",
            db,
            &new_etag
        )
        .execute(pool)
        .await?;
    }

    Ok(())
}
