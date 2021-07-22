//! Release file generation module

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use async_compression::tokio::write::{GzipEncoder, XzEncoder};
use log::{error, info, warn};
use nom::bytes::complete::{tag, take_until};
use nom::sequence::preceded;
use nom::IResult;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use sailfish::TemplateOnce;
use serde_json::Value;
use sqlx::PgPool;
use time::prelude::*;
use tokio::fs::{create_dir_all, metadata, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::spawn_blocking;

use crate::config::ReleaseConfig;
use crate::scan::{mtime, sha256sum};
use crate::sign::{load_certificate, sign_message};

const DEB822_DATE: &str = "%a, %d %b %Y %H:%M:%S %z";

struct PackageTemplate {
    name: String,
    version: String,
    section: Option<String>,
    arch: Option<String>,
    inst_size: Option<i64>,
    maintainer: Option<String>,
    path: Option<String>,
    size: Option<i64>,
    sha256: Option<String>,
    description: Option<String>,
    dep: Option<Value>,
}

#[derive(TemplateOnce)]
#[template(path = "Packages.stpl")]
struct PackagesTemplate {
    packages: Vec<PackageTemplate>,
}

#[derive(TemplateOnce)]
#[template(path = "InRelease.stpl")]
struct InReleaseTemplate {
    origin: String,
    label: String,
    codename: String,
    suite: String,
    description: String,
    date: String,
    valid_until: String,
    architectures: Vec<String>,
    components: Vec<String>,
    files: Vec<(String, u64, String)>,
}

struct BranchMeta {
    branch: String,
    arch: Option<Vec<String>>,
    comp: Option<Vec<String>>,
}

fn match_valid_until(input: &[u8]) -> IResult<&[u8], &[u8]> {
    tag("Valid-Until: ")(input)
}

fn skip_header(input: &[u8]) -> IResult<&[u8], &[u8]> {
    take_until("Valid-Until:")(input)
}

fn skip_other(input: &[u8]) -> IResult<&[u8], &[u8]> {
    preceded(skip_header, match_valid_until)(input)
}

fn parse_valid_date(input: &[u8]) -> IResult<&[u8], &[u8]> {
    preceded(skip_other, take_until("\n"))(input)
}

fn scan_single_release_file(branch_root: &Path, path: &Path) -> Result<(String, u64, String)> {
    use std::fs::File as StdFile;
    use std::io::Seek;
    use std::io::SeekFrom;

    let mut f = StdFile::open(path)?;
    let sha256 = sha256sum(&f)?;
    let filename = path.strip_prefix(branch_root)?.to_string_lossy();
    let length = f.seek(SeekFrom::Current(0))?;

    Ok((filename.to_string(), length, sha256))
}

fn scan_release_files(branch_root: &Path) -> Result<Vec<(String, u64, String)>> {
    let walk = walkdir::WalkDir::new(&branch_root).min_depth(2).into_iter();
    let mut files_to_scan = Vec::new();
    for entry in walk {
        let entry = entry?;
        if entry.file_type().is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        files_to_scan.push(entry.path().to_owned());
    }
    let files = files_to_scan
        .par_iter()
        .filter_map(|p| match scan_single_release_file(branch_root, p) {
            Ok(item) => Some(item),
            Err(e) => {
                error!("Error when scanning {}: {}", p.display(), e);
                None
            }
        })
        .collect::<Vec<_>>();

    Ok(files)
}

fn create_release_file(
    mirror_root: &Path,
    config: &ReleaseConfig,
    m: &BranchMeta,
    ttl: u64,
    cert: &Option<sequoia_openpgp::Cert>,
) -> Result<()> {
    use std::convert::TryInto;
    use std::fs::File as StdFile;

    info!("Generating InRelease files for {}", m.branch);

    let branch_root = mirror_root.join("dists").join(&m.branch);
    let release_files = scan_release_files(&branch_root);
    if let Err(e) = release_files {
        error!("Error when scanning {}: {}", m.branch, e);
        return Err(e);
    }
    let description = config
        .descriptions
        .get(&m.branch)
        .map_or_else(|| format!("AOSC OS Topic: {}", m.branch), |d| d.to_owned());
    let system_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let projected_timestamp = system_time + (ttl * 24 * 3600);
    let system_time = time::OffsetDateTime::from_unix_timestamp(system_time.try_into().unwrap());
    let projected_timestamp =
        time::OffsetDateTime::from_unix_timestamp(projected_timestamp.try_into().unwrap());

    let rendered = (InReleaseTemplate {
        origin: config.origin.clone(),
        label: config.label.clone(),
        codename: config.codename.clone(),
        suite: m.branch.clone(),
        description,
        date: system_time.format(DEB822_DATE),
        valid_until: projected_timestamp.format(DEB822_DATE),
        architectures: m.arch.as_ref().unwrap().to_vec(),
        components: m.comp.as_ref().unwrap().to_vec(),
        files: release_files.unwrap(),
    })
    .render_once();
    if let Err(e) = rendered {
        error!("Failed to generate release: {:?}", e);
        return Ok(());
    }
    let rendered = rendered.unwrap();
    if let Some(ref cert) = cert {
        // TODO: don't fail when signing failed
        let signed = sign_message(&cert, rendered.as_bytes())?;
        let mut f = StdFile::create(branch_root.join("InRelease"))?;
        f.write_all(&signed)?;
    } else {
        warn!("Certificate not found or not available. Release file not signed.");
        let mut f = StdFile::create(branch_root.join("Release"))?;
        f.write_all(rendered.as_bytes())?;
    }

    Ok(())
}

fn create_release_files(
    mirror_root: &Path,
    config: &ReleaseConfig,
    meta: &[BranchMeta],
    ttl: u64,
) -> Result<()> {
    let cert = if let Some(cert) = &config.cert {
        info!("Signing release files using certificate: {}", cert);
        Some(load_certificate(cert)?)
    } else {
        None
    };

    meta.par_iter().for_each_with(cert, |cert, meta| {
        if let Err(e) = create_release_file(mirror_root, config, meta, ttl, cert) {
            warn!("Failed to create release file: {}", e);
        }
    });

    Ok(())
}

async fn get_branch_metadata(pool: &PgPool) -> Result<Vec<BranchMeta>> {
    Ok(sqlx::query_as!(BranchMeta, "SELECT branch, array_agg(DISTINCT architecture) AS arch, array_agg(DISTINCT component) AS comp FROM pv_repos GROUP BY branch").fetch_all(pool).await?)
}

pub async fn render_releases(
    pool: &PgPool,
    mirror_root: &Path,
    config: ReleaseConfig,
    regenerate_list: &[String],
) -> Result<()> {
    let mut regenerate_set = HashSet::new();
    regenerate_set.reserve(regenerate_list.len());
    for r in regenerate_list {
        regenerate_set.insert(r);
    }

    let branches = get_branch_metadata(pool).await?;
    let branches = branches
        .into_iter()
        .filter(|branch| regenerate_set.contains(&branch.branch))
        .collect::<Vec<_>>();
    let mirror_root = mirror_root.to_owned();
    spawn_blocking(move || create_release_files(&mirror_root, &config, &branches, 10)).await??;

    Ok(())
}

async fn render_contents_in_component_arch(
    pool: &PgPool,
    component: &str,
    arch: String,
    component_root: &Path,
) -> Result<()> {
    let lines = sqlx::query!(
        r#"SELECT (df.path || '/' || df.name) || '   ' || (string_agg(DISTINCT (
coalesce(dp.section || '/', '') || dp.package), ',')) || chr(10) as p
FROM pv_packages dp
INNER JOIN pv_package_files df USING (package, version, repo)
INNER JOIN pv_repos pr ON pr.name=dp.repo
WHERE pr.path=$1 AND df.ftype=0
AND pr.architecture IN ($2, 'all') AND dp.debtime IS NOT NULL
GROUP BY df.path, df.name"#,
        component,
        arch
    )
    .fetch_all(pool)
    .await?;

    let content = lines
        .into_iter()
        .map(|line| line.p)
        .flatten()
        .collect::<String>();
    let dist_path = component_root.join(format!("Contents-{}.gz", arch));
    let mut f = GzipEncoder::new(File::create(dist_path).await?);
    f.write_all(content.as_bytes()).await?;
    f.shutdown().await?;

    Ok(())
}

pub async fn render_contents_in_component(
    pool: &PgPool,
    component: &str,
    mirror_root: &Path,
) -> Result<()> {
    info!("Generating Contents for {}", component);

    let records = sqlx::query!(
        "SELECT architecture FROM pv_repos WHERE architecture != 'all' AND path=$1",
        component
    )
    .fetch_all(pool)
    .await?;
    let component_root = mirror_root.join("dists").join(component);
    create_dir_all(&component_root).await?;

    let mut tasks = Vec::new();
    for record in records {
        tasks.push(render_contents_in_component_arch(
            pool,
            component,
            record.architecture,
            &component_root,
        ));
    }
    let results = futures::future::join_all(tasks).await;
    let mut errored = false;
    for result in results {
        if let Err(e) = result {
            errored = true;
            error!("Error generating contents: {}", e);
        }
    }
    if errored {
        return Err(anyhow!("One or more generation tasks returned an error"));
    }

    Ok(())
}

async fn render_packages_in_component_arch(
    arch: &str,
    packages: Vec<PackageTemplate>,
    component_root: &Path,
) -> Result<()> {
    let dist_path = component_root.join(format!("binary-{}", arch));
    create_dir_all(&dist_path).await?;
    let mut package_file = File::create(dist_path.join("Packages")).await?;
    let mut package_file_xz = XzEncoder::new(File::create(dist_path.join("Packages.xz")).await?);
    let rendered = spawn_blocking(move || PackagesTemplate { packages }.render_once()).await??;
    let results = tokio::join!(
        package_file.write_all(rendered.as_bytes()),
        package_file_xz.write_all(rendered.as_bytes())
    );
    // Raise an error if any
    results.0?;
    results.1?;
    // flush compressor cache
    package_file_xz.shutdown().await?;

    Ok(())
}

pub async fn render_packages_in_component(
    pool: &PgPool,
    component: &str,
    mirror_root: &Path,
) -> Result<()> {
    info!("Generating Packages for {}", component);

    let records = sqlx::query_as!(
        PackageTemplate,
        r#"SELECT p.package AS name, p.version, min(p.architecture) arch,
    min(p.filename) path, min(p.size) size, min(p.sha256) sha256,
    min(p.section) section, min(p.installed_size) inst_size,
    min(p.maintainer) maintainer, min(p.description) description,
    json_agg(array[pd.relationship, pd.value]) dep
FROM pv_packages p INNER JOIN pv_repos r ON p.repo=r.name
LEFT JOIN pv_package_dependencies pd ON pd.package=p.package
AND pd.version=p.version AND pd.repo=p.repo
WHERE r.path=$1 AND p.debtime IS NOT NULL
GROUP BY p.package, p.version, p.repo"#,
        component
    )
    .fetch_all(pool)
    .await?;

    let mut grouped_packages: HashMap<String, Vec<PackageTemplate>> = HashMap::new();
    for record in records {
        let arch_packages = grouped_packages.get_mut(record.arch.as_ref().unwrap());
        if let Some(arch_packages) = arch_packages {
            arch_packages.push(record);
        } else {
            let arch = record.arch.as_ref().unwrap().to_string();
            grouped_packages.insert(arch, vec![record]);
        }
    }

    let component_root = mirror_root.join("dists").join(component);
    for (arch, packages) in grouped_packages.into_iter() {
        render_packages_in_component_arch(&arch, packages, &component_root).await?;
    }

    Ok(())
}

/// Check if the branch needs refreshing. TTL is in days.
async fn need_refresh(inrel_path: &Path) -> Result<bool> {
    let mut f = File::open(inrel_path).await?;
    let mut content = Vec::new();
    f.read_to_end(&mut content).await?;
    let captured = parse_valid_date(&content).map_err(|e| anyhow!(e.to_string()))?;
    let captured_str = std::str::from_utf8(&captured.1)?;
    let parsed: time::OffsetDateTime =
        time::parse(captured_str, DEB822_DATE).map_err(|e| anyhow!(e))?;
    let parsed_timestamp = parsed.to_offset(offset!(+0)).unix_timestamp();
    let system_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let projected_timestamp = system_time + (24 * 3600);

    Ok(projected_timestamp >= parsed_timestamp as u64)
}

pub async fn need_regenerate(pool: &PgPool, mirror_root: &Path) -> Result<Vec<String>> {
    let dist_path = mirror_root.join("dists");
    let mut needs_regenerate = Vec::new();
    let records = sqlx::query!(
        "SELECT branch, coalesce(extract(epoch FROM max(mtime)), 0) AS modified FROM pv_repos GROUP BY branch"
    )
    .fetch_all(pool)
    .await?;
    for record in records {
        let inrelease_path = dist_path.join(&record.branch).join("InRelease");
        let inrelease_info = metadata(&inrelease_path).await;
        if let Ok(metadata) = inrelease_info {
            let mtime = mtime(&metadata).unwrap_or(0);
            if let Some(modified) = record.modified {
                if mtime >= modified as u64 && !need_refresh(&inrelease_path).await.unwrap_or(true)
                {
                    continue;
                }
            }
        }
        needs_regenerate.push(record.branch);
    }

    Ok(needs_regenerate)
}

#[test]
fn test_date_parsing() {
    let test_date = "Wed, 14 Jul 2021 10:54:24 +0000";
    let expected = date!(2021 - 07 - 14)
        .with_time(time!(10:54:24))
        .assume_utc();
    let parsed: time::OffsetDateTime = time::parse(test_date, DEB822_DATE).unwrap();
    assert_eq!(parsed, expected);
}

#[test]
fn test_inrel_parsing() {
    let test_data = r#"Origin: AOSC
Label: AOSC OS
Suite: bat-0.18.2
Codename: Hotfix
Description: AOSC OS Topic: bat-0.18.2
Date: Wed, 14 Jul 2021 10:54:24 +0000
Valid-Until: Sat, 24 Jul 2021 10:54:24 +0000
Architectures: amd64 arm64 loongson3 ppc64el"#;
    let captured = parse_valid_date(test_data.as_bytes()).unwrap();
    assert_eq!(captured.1, &b"Sat, 24 Jul 2021 10:54:24 +0000"[..]);
}
