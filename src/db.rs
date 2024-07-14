use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use log::{error, info};
use sqlx::{Executor, PgPool};

const PV_RS_SQL_SCRIPT_PV: &str = include_str!("../migrations/20210621205620_pv-base.down.sql");
const PV_RS_SQL_SCRIPT_AB: &str = include_str!("../migrations/20210621205247_abbsdb-base.down.sql");

pub struct PVPackage {
    pub package: Option<String>,
    pub version: Option<String>,
    pub repo: Option<String>,
    pub architecture: Option<String>,
    pub filename: Option<String>,
    pub size: Option<i64>,
    pub mtime: Option<i32>,
    pub sha256: Option<String>,
}

/// Run all the pending migrations in `migrations` directory
pub async fn run_migrate(pool: &PgPool) -> Result<()> {
    Ok(sqlx::migrate!().run(pool).await?)
}

/// Connect to the database
pub async fn connect_database(connspec: &str) -> Result<PgPool> {
    Ok(PgPool::connect(connspec).await?)
}

/// Run database maintenance
pub async fn run_maintenance(pool: &PgPool) -> Result<()> {
    info!("Refreshing materialized views ... ");
    if let Err(e) = refresh_views(pool).await {
        error!("Error refreshing views: {}", e);
    }
    // vacuum the database
    info!("Running database garbage collection ...");
    sqlx::query!("VACUUM ANALYZE").execute(pool).await?;

    Ok(())
}

/// Erase everything
pub async fn reset_database(pool: &PgPool) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query!("TRUNCATE TABLE _sqlx_migrations")
        .execute(&mut *tx)
        .await?;
    info!("Resetting p-vector tables ...");
    tx.execute(PV_RS_SQL_SCRIPT_PV).await?;
    info!("Resetting abbs sync tables ...");
    tx.execute(PV_RS_SQL_SCRIPT_AB).await?;
    tx.commit().await?;
    info!("Running database garbage collection ...");
    sqlx::query!("VACUUM").execute(pool).await?;
    info!("Reset done.");

    Ok(())
}

/// List all the packages in a specific component (branch)
pub async fn list_packages_in_component(pool: &PgPool, component: &str) -> Result<Vec<PVPackage>> {
    let records = sqlx::query_as!(
        PVPackage,
r#"SELECT p.package, p.version, p.repo, p.architecture, p.filename, p.size, p.mtime, p.sha256
FROM pv_packages p INNER JOIN pv_repos r ON p.repo=r.name WHERE r.path=$1
UNION ALL
SELECT p.package, p.version, p.repo, p.architecture, p.filename, p.size, p.mtime, p.sha256
FROM pv_package_duplicate p INNER JOIN pv_repos r ON p.repo=r.name WHERE r.path=$1"#,
        component
    ).fetch_all(pool).await?;

    Ok(records)
}

/// Generate notifying messages for removed packages
pub async fn get_removed_packages_message<P: AsRef<Path>>(
    pool: &PgPool,
    path: &[P],
) -> Result<Vec<crate::ipc::PVMessage>> {
    let mut messages = Vec::new();
    for path in path {
        let path = path.as_ref().to_string_lossy();
        let record = sqlx::query!(
            "SELECT package, version, r.branch || '-' || r.component AS repo, p.architecture FROM pv_packages p JOIN pv_repos r ON p.repo = r.name WHERE filename = $1",
            path.as_ref()
        )
        .fetch_one(pool)
        .await?;
        messages.push(crate::ipc::PVMessage::new(
            record.repo.unwrap_or_else(|| "?".to_string()),
            record.package,
            record.architecture,
            b'-',
            Some(record.version),
            None,
        ))
    }

    Ok(messages)
}

pub async fn remove_packages_by_path<P: AsRef<Path>>(pool: &PgPool, path: &[P]) -> Result<()> {
    let mut tx = pool.begin().await?;
    let mut changed_repos = HashSet::new();
    for path in path {
        let path = path.as_ref().to_string_lossy();
        let p = sqlx::query!(
            "DELETE FROM pv_packages WHERE filename = $1 RETURNING repo",
            path.as_ref()
        )
        .fetch_one(&mut *tx)
        .await?;
        changed_repos.insert(p.repo);
    }
    for b in changed_repos {
        sqlx::query!("UPDATE pv_repos SET mtime=now() WHERE name = $1", b)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    Ok(())
}

/// Refresh materialized views
pub async fn refresh_views(pool: &PgPool) -> Result<()> {
    sqlx::query!("REFRESH MATERIALIZED VIEW CONCURRENTLY v_packages_new")
        .execute(pool)
        .await?;
    sqlx::query!("REFRESH MATERIALIZED VIEW CONCURRENTLY v_dpkg_dependencies")
        .execute(pool)
        .await?;
    sqlx::query!("REFRESH MATERIALIZED VIEW CONCURRENTLY v_so_breaks")
        .execute(pool)
        .await?;
    sqlx::query!("REFRESH MATERIALIZED VIEW CONCURRENTLY v_so_breaks_dep")
        .execute(pool)
        .await?;

    Ok(())
}

/// Load sqlite_fdw extension (external binary)
pub async fn load_fdw_ext(pool: &PgPool) -> Result<()> {
    sqlx::query!("CREATE EXTENSION IF NOT EXISTS sqlite_fdw")
        .execute(pool)
        .await?;

    Ok(())
}
