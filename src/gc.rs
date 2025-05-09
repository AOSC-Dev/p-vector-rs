//! Garbage collection module

use std::path::Path;

use anyhow::Result;
use log::{error, info};
use sqlx::PgPool;
use tokio::fs::{remove_dir, remove_dir_all, remove_file};

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
            if let Err(e) = remove_dir_all(&path).await {
                error!("Failed to remove \"{}\": {}", remove, e);
            }
            if let Some(p) = path.parent() {
                // remove the inrelease file
                remove_file(p.join("InRelease")).await.ok();
                // remove the parent directory if it's empty
                remove_dir(p).await.ok();
            }
        });
    }
    futures::future::join_all(tasks).await;
}

async fn clean_removed_main_branches(pool: &PgPool) -> Result<()> {
    sqlx::query!(
        "WITH deleted_branches AS (
    SELECT r.name FROM pv_repos r
    LEFT JOIN pv_packages p ON p.repo = r.name
    GROUP BY r.name HAVING COUNT(DISTINCT p.package) < 1
)
DELETE FROM pv_repos USING deleted_branches
WHERE pv_repos.name = deleted_branches.name"
    )
    .execute(pool)
    .await?;

    Ok(())
}

fn clean_by_hash_files_inner(byhash_path: &Path, files_to_keep: usize) -> Result<()> {
    let mut byhash_files = Vec::new();
    for entry in walkdir::WalkDir::new(&byhash_path) {
        if let Ok(entry) = entry {
            if !entry.file_type().is_file() {
                continue;
            }
            byhash_files.push((entry.metadata()?.modified()?, entry.path().to_path_buf()));
        }
    }

    let num_files = byhash_files.len();
    if num_files <= files_to_keep {
        return Ok(());
    }
    byhash_files.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    for i in 0..(num_files - files_to_keep) {
        let (_, path) = &byhash_files[i];
        if let Err(e) = std::fs::remove_file(path) {
            error!("Failed to remove by-hash file {}: {}", path.display(), e);
        }
    }

    Ok(())
}

pub fn clean_by_hash_files(branch_root: &Path, copies_to_keep: isize) -> Result<()> {
    if copies_to_keep < 0 {
        return Ok(());
    }

    let mut last_count = 0usize;
    // try to guess how many files should be kept
    for entry in walkdir::WalkDir::new(&branch_root) {
        if let Ok(entry) = entry {
            if !entry.file_type().is_file() {
                continue;
            }
            if !entry
                .path()
                .parent()
                .map(|p| p.ends_with("by-hash/SHA256"))
                .unwrap_or_default()
            {
                last_count += 1;
            }
        }
    }

    let files_to_keep = last_count * (copies_to_keep as usize);
    // execute the clean-up
    for entry in walkdir::WalkDir::new(&branch_root) {
        if let Ok(entry) = entry {
            if !entry.file_type().is_dir() {
                continue;
            }
            if entry.path().ends_with("by-hash/SHA256") {
                clean_by_hash_files_inner(entry.path(), files_to_keep)?;
            }
        }
    }

    Ok(())
}

/// Execute garbage collection
pub async fn run_gc<P: AsRef<Path>>(pool: &PgPool, mirror_root: P) -> Result<()> {
    info!("Deleting duplicated and stale entries from the database ...");
    sqlx::query!("DELETE FROM pv_package_duplicate USING pv_packages WHERE pv_package_duplicate.filename = pv_packages.filename").execute(pool).await?;
    clean_removed_main_branches(pool).await?;
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
        info!("No stale branch to remove.");
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
    clean_dist_files(&to_remove, mirror_root.as_ref()).await;
    info!("GC finished.");

    Ok(())
}
