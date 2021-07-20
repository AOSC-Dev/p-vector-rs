use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::Result;
use futures::future::Either;
use log::{error, info};
use sqlx::PgPool;
use tokio::task::{block_in_place, spawn_blocking};
use walkdir::DirEntry;

use crate::scan::collect_removed_packages;

mod cli;
mod config;
mod db;
mod gc;
mod generate;
mod parser;
mod scan;
mod sign;
mod sync;

macro_rules! log_error {
    ($i:expr, $stage:expr) => {
        if let Err(err) = $i {
            error!("Error while {}: {}", $stage, err);
        }
    };
}

async fn list_all_packages(pool: &PgPool, components: &[PathBuf]) -> Result<Vec<db::PVPackage>> {
    let mut results = Vec::new();
    for component in components {
        let name = component.to_string_lossy();
        results.extend(db::list_packages_in_component(pool, &name).await?);
    }

    Ok(results)
}

fn get_changed_packages<'a>(discovered: &'a [DirEntry], scanned: &[PathBuf]) -> Vec<&'a Path> {
    let mut scanned_cache = HashSet::new();
    let mut changed = Vec::new();
    for entry in scanned {
        scanned_cache.insert(entry.as_path());
    }
    for directory in discovered {
        if scanned_cache.contains(directory.path()) {
            continue;
        }
        changed.push(directory.path());
    }

    changed
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: cli::PVector = argh::from_env();
    env_logger::init();

    let config = config::parse_config(args.config.as_str())?;
    config::lint_config(&config);

    info!("Connecting to database...");
    let pool = db::connect_database(&config.config.db_pgconn).await?;
    info!("Running any pending migrations...");
    db::run_migrate(&pool).await?;

    match args.command {
        cli::PVectorCommand::Scan(_) => scan_action(config, &pool).await?,
        cli::PVectorCommand::Release(_) => release_action(&config, &pool).await?,
        cli::PVectorCommand::Sync(_) => sync::sync_db_updates(&pool).await?,
        cli::PVectorCommand::Analyze(_) => {
            analysis_action(&pool, config.config.qa_interval).await?
        }
        cli::PVectorCommand::Reset(_) => todo!(),
        cli::PVectorCommand::GC(_) => gc_action(&config, &pool).await?,
        cli::PVectorCommand::Full(_) => full_action(config, &pool).await?,
    }

    Ok(())
}

async fn full_action(config: config::Config, pool: &PgPool) -> Result<()> {
    scan_action(config.clone(), pool).await?;
    let stage1_results = tokio::join!(sync::sync_db_updates(pool), gc_action(&config, pool));
    let stage2_results = tokio::join!(
        analysis_action(pool, config.config.qa_interval),
        release_action(&config, pool)
    );
    log_error!(stage1_results.0, "synchronizing database");
    log_error!(stage1_results.1, "garbage collecting");
    log_error!(stage2_results.0, "analyzing issues");
    log_error!(stage2_results.1, "generating release files");

    Ok(())
}

async fn analysis_action(pool: &PgPool, delay: usize) -> Result<()> {
    info!("Running analysis ...");
    db::run_analysis(pool, delay).await?;
    info!("Analysis completed.");

    Ok(())
}

async fn gc_action(config: &config::Config, pool: &PgPool) -> Result<()> {
    let mirror_root = Path::new(&config.config.path);
    gc::run_gc(pool, mirror_root).await?;

    Ok(())
}

async fn release_action(config: &config::Config, pool: &PgPool) -> Result<()> {
    let mirror_root = Path::new(&config.config.path);
    let pool_path = Path::new(&config.config.path).join("pool");
    let topics = spawn_blocking(move || scan::discover_topics_components(&pool_path)).await??;
    info!("{} topics discovered.", topics.len());
    let needs_regenerate = generate::need_regenerate(pool, mirror_root).await?;
    let mut tasks = Vec::new();
    for topic in topics {
        let mut skip = true;
        for t in needs_regenerate.iter() {
            if topic.starts_with(t) {
                skip = false;
                break;
            }
        }
        if skip {
            info!("Skipping {}", topic.display());
            continue;
        }
        let name = topic.to_string_lossy().to_string();
        let name_clone = name.clone();
        tasks.push(Either::Left(async move {
            generate::render_packages_in_component(pool, &name, mirror_root).await
        }));
        tasks.push(Either::Right(async move {
            generate::render_contents_in_component(pool, &name_clone, mirror_root).await
        }));
    }
    let results = futures::future::join_all(tasks).await;
    for result in results {
        log_error!(result, "generating manifest");
    }
    let release_config = config::convert_branch_description_config(&config);
    generate::render_releases(pool, mirror_root, release_config).await?;
    info!("Generation finished.");

    Ok(())
}

async fn scan_action(config: config::Config, pool: &PgPool) -> Result<()> {
    let pool_path = Path::new(&config.config.path).join("pool");
    let mirror_root = config.config.path.clone();
    let mirror_root_clone = Path::new(&mirror_root).to_owned();
    let topics = spawn_blocking(move || scan::discover_topics_components(&pool_path)).await??;
    info!("{} topics discovered.", topics.len());
    let files = spawn_blocking(move || scan::collect_all_packages(&config.config.path)).await??;
    info!("{} deb files discovered.", files.len());
    info!("Collecting packages information from database ...");
    let db_packages = list_all_packages(&pool, &topics).await?;
    info!("Database knows {} packages.", db_packages.len());
    info!("Pre-scanning packages to determine which packages are different ...");
    let (delete, scanned) =
        block_in_place(move || scan::validate_packages(mirror_root, &db_packages))?;
    let changed = get_changed_packages(&files, &scanned);
    info!(
        "{} up to date, {} deleted, {} changed.",
        scanned.len(),
        delete.len(),
        changed.len()
    );
    info!("Starting scanner ...");
    let mirror_root = mirror_root_clone.clone();
    let packages =
        block_in_place(move || scan::scan_packages_advanced(&changed, &mirror_root_clone));
    info!("Scan finished.");
    let deleted = collect_removed_packages(delete, &mirror_root);
    info!("Deleting {} packages from database ...", deleted.len());
    db::remove_packages_by_path(pool, &deleted).await?;
    info!("Saving changes to database ...");
    scan::update_changed_repos(pool, &packages).await?;
    scan::save_packages_to_db(pool, &packages).await?;
    info!("Saving completed.");

    Ok(())
}
