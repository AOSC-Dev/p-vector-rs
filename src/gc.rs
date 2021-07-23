//! Garbage collection module

use std::path::Path;

use anyhow::Result;
use log::{error, info};
use sqlx::PgPool;
use tokio::fs::remove_dir_all;

/// List all the known branches in the database
async fn list_existing_branches(pool: &PgPool) -> Result<Vec<String>> {
    let records = sqlx::query!("SELECT DISTINCT path FROM pv_repos")
        .fetch_all(pool)
        .await?;
    let results = records.into_iter().map(|x| x.path).collect::<Vec<_>>();

    Ok(results)
}

async fn clean_dist_files(to_remove: &[&String], mirror_root: &Path) {
    let mut tasks = Vec::new();
    for remove in to_remove {
        tasks.push(async move {
            info!("Deleting dists: {} ...", remove);
            let path = mirror_root.join("dists").join(remove);
            if let Err(e) = remove_dir_all(path).await {
                error!("Failed to remove \"{}\": {}", remove, e);
            }
        });
    }
    futures::future::join_all(tasks).await;
}

async fn refresh_views(pool: &PgPool) -> Result<()> {
    tokio::try_join!(
        sqlx::query!("REFRESH MATERIALIZED VIEW v_packages_new").execute(pool),
        sqlx::query!("REFRESH MATERIALIZED VIEW v_dpkg_dependencies").execute(pool),
        sqlx::query!("REFRESH MATERIALIZED VIEW v_so_breaks").execute(pool),
        sqlx::query!("REFRESH MATERIALIZED VIEW v_so_breaks_dep").execute(pool),
    )?;

    Ok(())
}

/// Execute garbage collection
pub async fn run_gc<P: AsRef<Path>>(pool: &PgPool, mirror_root: P) -> Result<()> {
    let known_branches = list_existing_branches(pool).await?;
    let to_remove = known_branches
        .iter()
        .filter(|branch| {
            let path = mirror_root.as_ref().join("pool").join(branch);

            !path.is_dir()
        })
        .collect::<Vec<_>>();
    // exit early if no changes
    if to_remove.is_empty() {
        info!("Nothing to do.");
        return Ok(());
    }
    info!(
        "Database knows {} branches, {} of which will be removed.",
        known_branches.len(),
        to_remove.len()
    );
    for remove in to_remove.iter() {
        info!("Deleting from database: {} ...", remove);
        sqlx::query!("DELETE FROM pv_repos WHERE path = $1", remove)
            .execute(pool)
            .await?;
    }
    let result = tokio::join!(
        clean_dist_files(&to_remove, mirror_root.as_ref()),
        refresh_views(&pool)
    );
    if let Err(e) = result.1 {
        error!("Error refreshing views: {}", e);
    }

    Ok(())
}
