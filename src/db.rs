use std::path::Path;

use anyhow::Result;
use log::info;
use sqlx::{Executor, PgPool};

const PV_QA_SQL_SCRIPT: &str = include_str!("../sql/pkgissues.sql");
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

/// Run QA analysis
pub async fn run_analysis(pool: &PgPool, delay: usize) -> Result<()> {
    let mut tx = pool.acquire().await?;
    let stmt = format!(
        "SELECT max(atime) + INTERVAL '{} hours' >= now() AS refresh FROM pv_package_issues",
        delay
    );
    let refresh: Option<bool> = sqlx::query_scalar(&stmt).fetch_one(&mut tx).await?;
    if refresh.unwrap_or(false) {
        info!("Analysis skipped.");
        return Ok(());
    }
    // unprepared transaction is used since this is a SQL script file
    tx.execute(PV_QA_SQL_SCRIPT).await?;

    Ok(())
}

/// Erase everything
pub async fn reset_database(pool: &PgPool) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query!("TRUNCATE TABLE _sqlx_migrations")
        .execute(&mut tx)
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
            "SELECT package, version, repo, architecture FROM pv_packages WHERE filename = $1",
            path.as_ref()
        )
        .fetch_one(pool)
        .await?;
        messages.push(crate::ipc::PVMessage::new(
            record.repo,
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
    for path in path {
        let path = path.as_ref().to_string_lossy();
        sqlx::query!("DELETE FROM pv_packages WHERE filename = $1", path.as_ref())
            .execute(&mut tx)
            .await?;
    }
    tx.commit().await?;

    Ok(())
}

/// Refresh materialized views
pub async fn refresh_views(pool: &PgPool) -> Result<()> {
    tokio::try_join!(
        sqlx::query!("REFRESH MATERIALIZED VIEW v_packages_new").execute(pool),
        sqlx::query!("REFRESH MATERIALIZED VIEW v_dpkg_dependencies").execute(pool),
        sqlx::query!("REFRESH MATERIALIZED VIEW v_so_breaks").execute(pool),
        sqlx::query!("REFRESH MATERIALIZED VIEW v_so_breaks_dep").execute(pool),
    )?;

    Ok(())
}

/// Load sqlite_fdw extension (external binary)
pub async fn load_fdw_ext(pool: &PgPool) -> Result<()> {
    sqlx::query!("CREATE EXTENSION IF NOT EXISTS sqlite_fdw")
        .execute(pool)
        .await?;

    Ok(())
}
