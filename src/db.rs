use anyhow::Result;
use sqlx::{Executor, PgPool};

const PV_QA_SQL_SCRIPT: &str = include_str!("../sql/pkgissues.sql");

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
pub async fn run_analysis(pool: &PgPool) -> Result<()> {
    let mut tx = pool.acquire().await?;
    // unprepared transaction is used since this is a SQL script file
    tx.execute(PV_QA_SQL_SCRIPT).await?;

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

pub async fn remove_packages_by_path<S: AsRef<str>>(pool: &PgPool, path: &[S]) -> Result<()> {
    let mut tx = pool.begin().await?;
    for path in path {
        sqlx::query!("DELETE FROM pv_packages WHERE filename = $1", path.as_ref())
            .execute(&mut tx)
            .await?;
    }
    tx.commit().await?;

    Ok(())
}

/// Load sqlite_fdw extension (external binary)
pub async fn load_fdw_ext(pool: &PgPool) -> Result<()> {
    sqlx::query!("CREATE EXTENSION IF NOT EXISTS sqlite_fdw")
        .execute(pool)
        .await?;

    Ok(())
}
