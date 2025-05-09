use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Result;
use futures::future::Either;
use log::{error, info};
use sqlx::PgPool;
use time::macros::format_description;
use tokio::{
    task::{block_in_place, spawn_blocking},
    time::sleep,
};
use walkdir::DirEntry;

#[cfg(feature = "systemd")]
use sd_notify::NotifyState;

use crate::scan::collect_removed_packages;

mod cli;
mod config;
mod db;
mod gc;
mod generate;
mod ipc;
mod parser;
mod scan;
mod sign;

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

    #[cfg(feature = "systemd")]
    {
        sd_notify::notify(true, &[NotifyState::Ready]).ok();
    }

    match args.command {
        cli::PVectorCommand::Scan(_) => scan_action(config, &pool).await?,
        cli::PVectorCommand::Release(_) => release_action(&config, &pool).await?,
        cli::PVectorCommand::Maintenance(_) => maintenance_action(&pool).await?,
        cli::PVectorCommand::Reset(_) => reset_action(&pool).await?,
        cli::PVectorCommand::GC(_) => gc_action(&config, &pool).await?,
        cli::PVectorCommand::Full(_) => full_action(config, &pool).await?,
        cli::PVectorCommand::GenKey(_) => generate_key(args.config.as_str()).await?,
    }

    Ok(())
}

async fn full_action(config: config::Config, pool: &PgPool) -> Result<()> {
    scan_action(config.clone(), pool).await?;
    let gc_result = gc_action(&config, pool).await;
    let stage2_results = tokio::join!(maintenance_action(pool), release_action(&config, pool));
    log_error!(gc_result, "garbage collecting");
    log_error!(stage2_results.0, "database maintenance");
    log_error!(stage2_results.1, "generating release files");

    Ok(())
}

async fn maintenance_action(pool: &PgPool) -> Result<()> {
    info!("Running database maintenance ...");
    db::run_maintenance(pool).await?;
    info!("Maintenance completed.");

    Ok(())
}

async fn gc_action(config: &config::Config, pool: &PgPool) -> Result<()> {
    let mirror_root = Path::new(&config.config.path).to_path_buf();
    gc::run_gc(pool, &mirror_root).await?;
    if let Some(acquire_by_hash) = &config.config.acquire_by_hash {
        let acquire_by_hash = *acquire_by_hash;
        tokio::task::spawn_blocking(move || {
            for i in walkdir::WalkDir::new(mirror_root.join("dists"))
                .min_depth(1)
                .max_depth(1)
            {
                if let Ok(entry) = i {
                    if entry.file_type().is_dir() {
                        gc::clean_by_hash_files(entry.path(), acquire_by_hash).ok();
                    }
                }
            }
        })
        .await?;
    }

    Ok(())
}

async fn release_action(config: &config::Config, pool: &PgPool) -> Result<()> {
    let mirror_root = Path::new(&config.config.path);
    let pool_path = Path::new(&config.config.path).join("pool");
    let topics = spawn_blocking(move || scan::discover_topics_components(pool_path)).await??;
    info!("{} topics discovered.", topics.len());
    let needs_regenerate = generate::need_regenerate(pool, mirror_root).await?;
    let mut tasks = Vec::new();
    let tempdir = tempfile::tempdir()?;
    let tempdir_path = tempdir.path().to_owned();
    let use_acquire_by_hash = config
        .config
        .acquire_by_hash
        .map(|v| v != 0)
        .unwrap_or_default();
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
        let tempdir_path = tempdir_path.clone();
        let tempdir_path_clone = tempdir_path.clone();
        tasks.push(Either::Left(async move {
            generate::render_packages_in_component(pool, &name, &tempdir_path).await
        }));
        tasks.push(Either::Right(async move {
            generate::render_contents_in_component(pool, &name_clone, &tempdir_path_clone).await
        }));
    }
    let results = futures::future::join_all(tasks).await;
    for result in results {
        log_error!(result, "generating manifest");
    }
    let release_config = config::convert_branch_description_config(config);
    let mirror_root = mirror_root.to_owned();
    let mirror_root_clone = mirror_root.clone();
    spawn_blocking(move || {
        let new_dists = tempdir_path.join("dists");
        if !new_dists.exists() {
            info!("No new dists generated.");
            return Ok(0);
        }
        fs_extra::dir::move_dir(
            tempdir_path.join("dists"),
            &mirror_root,
            &fs_extra::dir::CopyOptions {
                overwrite: true,
                ..Default::default()
            },
        )
    })
    .await??;
    generate::render_releases(
        pool,
        &mirror_root_clone,
        release_config,
        &needs_regenerate,
        use_acquire_by_hash,
    )
    .await?;
    info!("Generation finished.");

    Ok(())
}

async fn reset_action(pool: &PgPool) -> Result<()> {
    db::reset_database(pool).await
}

fn ask_for_key_info() -> Result<String> {
    use dialoguer::theme::ColorfulTheme;
    use dialoguer::Input;

    let name: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Your name")
        .interact_text()?;
    let email: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Your e-mail address")
        .interact_text()?;

    Ok(format!("{} <{}>", name, email))
}

async fn generate_key(config: &str) -> Result<()> {
    use secrecy::ExposeSecret;
    use time::OffsetDateTime;
    use tokio::fs::{create_dir_all, File};
    use tokio::io::AsyncWriteExt;

    let userid = spawn_blocking(ask_for_key_info).await??;
    let path =
        Path::new(&std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join("pv-keys");
    create_dir_all(&path).await?;
    let cert = spawn_blocking(move || sign::generate_certificate(&userid)).await??;
    let priv_path = path.join(format!("{}.key", cert.id));
    let pub_path = path.join(format!("{}.pub", cert.id));
    let mut p_file = File::create(&priv_path).await?;
    let mut c_file = File::create(&pub_path).await?;
    p_file
        .write_all(cert.privkey.expose_secret().as_ref())
        .await?;
    c_file
        .write_all(cert.pubkey.expose_secret().as_ref())
        .await?;
    let expiry = OffsetDateTime::from_unix_timestamp(cert.expiry.try_into().unwrap());
    let expiry_format = expiry?.format(&format_description!(
        "[year]-[month]-[day] [hour]:[minute] [offset_hour sign:mandatory]:[offset_minute]"
    ))?;
    let inst = sign::generate_instructions(
        pub_path.display().to_string(),
        priv_path.display().to_string(),
        expiry_format,
        config,
    )?;
    println!("\n{}", inst);

    Ok(())
}

async fn collect_package_changes(
    pool: &PgPool,
    packages: &[scan::PackageMeta],
    removed: &[PathBuf],
) -> Result<(Vec<ipc::PVMessage>, Vec<ipc::PVMessage>)> {
    let result = tokio::try_join!(
        scan::what_changed(pool, packages),
        db::get_removed_packages_message(pool, removed)
    )?;

    Ok(result)
}

async fn scan_action(config: config::Config, pool: &PgPool) -> Result<()> {
    let pool_path = Path::new(&config.config.path).join("pool");
    let pool_path_clone = pool_path.clone();
    let mirror_root = config.config.path.clone();
    let mirror_root_path = Path::new(&mirror_root).to_owned();
    let topics = spawn_blocking(move || scan::discover_topics_components(pool_path)).await??;
    info!("{} topics discovered.", topics.len());
    let files = spawn_blocking(move || scan::collect_all_packages(pool_path_clone)).await??;
    info!("{} deb files discovered.", files.len());
    info!("Collecting packages information from database ...");
    let db_packages = list_all_packages(pool, &topics).await?;
    info!("Database knows {} packages.", db_packages.len());
    info!("Pre-scanning packages to determine which packages are different ...");
    let (delete, scanned, needs_update) =
        block_in_place(move || scan::validate_packages(mirror_root, &db_packages))?;
    let changed = get_changed_packages(&files, &scanned);
    info!(
        "{} up to date, {} deleted, {} changed.",
        scanned.len(),
        delete.len(),
        changed.len()
    );
    if !needs_update.is_empty() {
        info!("{} packages needs metadata refresh.", needs_update.len());
        scan::update_unchanged_packages(pool, needs_update, &mirror_root_path).await?;
    }
    if delete.is_empty() && changed.is_empty() {
        info!("Nothing to scan.");
        return Ok(());
    }
    #[cfg(feature = "systemd")]
    {
        let changed_number = delete.len() + changed.len();
        let message = format!("Processing {} packages ...", changed_number);
        sd_notify::notify(true, &[NotifyState::Status(&message)]).ok();
    }
    info!("Starting scanner ...");
    let mirror_root = mirror_root_path.clone();
    let packages =
        block_in_place(move || scan::scan_packages_advanced(&changed, &mirror_root_path));
    info!("Scan finished.");
    let deleted = collect_removed_packages(delete, &mirror_root);
    // IPC operations
    // TODO: Move these to somewhere else maybe?
    ipc_publish(config, pool, &packages, &deleted).await?;
    info!("Deleting {} packages from database ...", deleted.len());
    db::remove_packages_by_path(pool, &deleted).await?;
    info!("Saving changes to database ...");
    scan::update_changed_repos(pool, &packages).await?;
    scan::save_packages_to_db(pool, &packages).await?;
    info!("Saving completed.");

    Ok(())
}

async fn ipc_publish(
    config: config::Config,
    pool: &PgPool,
    packages: &[scan::PackageMeta],
    deleted: &[PathBuf],
) -> Result<()> {
    if let Some(ref ipc_address) = config.config.change_notifier {
        let mut socket = ipc::redis_connect(ipc_address)?;
        // sleep 1 second so that the client is ready
        sleep(Duration::from_secs(1)).await;
        info!("Collecting changed packages ...");
        let (changed, removed) = collect_package_changes(pool, packages, deleted).await?;
        info!("Publishing changes to {} ...", ipc_address);
        spawn_blocking(move || -> Result<()> {
            ipc::publish_pv_messages(&removed, &mut socket)?;
            ipc::publish_pv_messages(&changed, &mut socket)?;
            Ok(())
        })
        .await??;
    }

    Ok(())
}
