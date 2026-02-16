//! Advanced database-based scanning.

use anyhow::{anyhow, Result};
use crossbeam_queue::SegQueue;
use log::{error, info, warn};
use rayon::prelude::*;
use sqlx::{PgPool, Postgres, Transaction};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use std::{fs::Metadata, path::Component};

use crate::db;
use crate::ipc::PVMessage;
use crate::scan::{determine_format, open_compressed_control, ArArchive, TarArchive};

use super::{mtime, read_compressed, HashedReader, TarFormat};

macro_rules! must_have {
    ($map:ident, $name:expr) => {{
        let value = $map
            .remove($name.as_bytes())
            .ok_or_else(|| anyhow!("Missing `{}` field", $name))?;
        std::str::from_utf8(value)?.to_string()
    }};
}

/// ELF magic number
const ELF_MAGIC: &[u8] = &[0x7f, 0x45, 0x4c, 0x46];
/// Deb relationships
const PKG_RELATION: &[&str] = &[
    "Depends",
    "Pre-Depends",
    "Recommends",
    "Suggests",
    "Enhances",
    "Breaks",
    "Conflicts",
    "Provides",
    "Replaces",
    "Multi-Arch",
];

#[derive(Debug)]
struct DebMeta {
    /// PKGNAME (Package)
    name: String,
    /// PKGVER (Version)
    version: String,
    /// PKGSEC (Section)
    section: String,
    /// PKGDES (Description)
    desc: String,
    /// Architecture
    arch: String,
    /// Installed-Size
    inst_size: String,
    /// Maintainer
    maintainer: String,
    /// Features
    features: Option<String>,
    // Utility fields
    /// control.tar last modified time
    debtime: u64,
    /// Extra metadata from control (e.g. relationship information)
    extra: HashMap<Vec<u8>, Vec<u8>>,
}

#[derive(Debug)]
pub struct PackageMeta {
    deb: DebMeta,
    /// Filename
    filename: String,
    /// Size
    size: u64,
    /// (SHA256)
    sha256: String,
    // Utility fields
    /// Repository name (branch, component)
    repo: (String, String),
    /// Last Modified time
    mtime: u64,
    /// Files contained in this package
    contents: PackageContents,
}

#[derive(Debug)]
struct PackageFile {
    path: PathBuf,
    is_dir: bool,
    size: u64,
    type_: u8,
    perms: u32,
    uid: u64,
    gid: u64,
    uname: Option<Vec<u8>>,
    gname: Option<Vec<u8>>,
}

#[derive(Debug)]
struct PackageContents {
    files: Vec<PackageFile>,
    so_provides: HashSet<String>,
    so_requires: HashSet<String>,
}

#[derive(Debug)]
struct RepositoryMeta {
    name: String,
    key: String,
    path: String,
    branch: String,
    component: String,
    architecture: String,
}

fn open_compressed_data<R: Read>(reader: R, format: &TarFormat) -> Result<PackageContents> {
    read_compressed(format, reader, collect_files)
}

/// Collect left-over fields from the hashmap
fn collect_left_over_fields(map: HashMap<&[u8], &[u8]>) -> HashMap<Vec<u8>, Vec<u8>> {
    let mut new_map: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    for (key, value) in map {
        new_map.insert(key.to_vec(), value.to_vec());
    }

    new_map
}

fn sha256sum_validate<P: AsRef<Path>>(file: P, expected: &str) -> Result<bool> {
    let f = File::open(file)?;
    let hash = super::sha256sum(f)?;

    Ok(hash == expected)
}

pub fn collect_removed_packages(removed: SegQueue<PathBuf>, mirror_root: &Path) -> Vec<PathBuf> {
    let mut removed_packages = Vec::with_capacity(removed.len());
    while let Some(package) = removed.pop() {
        removed_packages.push(package.strip_prefix(mirror_root).unwrap().to_path_buf());
    }

    removed_packages
}

/// Validate if the records in the database are up to date with the packages
pub fn validate_packages<P: AsRef<Path>>(
    root: P,
    packages: &[db::PVPackage],
) -> Result<(SegQueue<PathBuf>, Vec<PathBuf>, SegQueue<(PathBuf, u64)>)> {
    let pool_root = root.as_ref();
    let to_remove = SegQueue::new();
    let needs_update = SegQueue::new();
    let already_scanned = packages
        .par_iter()
        .filter_map(|p| {
            let path = pool_root.join(p.filename.as_ref().unwrap());
            if !path.exists() {
                to_remove.push(path);
                return None;
            }
            let stat = path.metadata();
            if let Err(e) = stat {
                warn!("Problem stat() on {}: {}", path.display(), e);
                return None;
            }
            let stat = stat.unwrap();
            if stat.is_file() {
                let size = p.size.unwrap();
                let mtime = super::mtime(&stat).unwrap_or(0);
                if size.is_negative() || stat.len() != (size as u64) {
                    // ^ ... what?
                    return None;
                }
                if mtime == p.mtime.unwrap_or(0) as u64 {
                    // mark as already scanned
                    return Some(path);
                } else if sha256sum_validate(&path, p.sha256.as_ref().unwrap()).unwrap_or(false) {
                    needs_update.push((path.clone(), mtime));
                    return Some(path);
                }
                return None;
            }
            to_remove.push(path);

            None
        })
        .collect::<Vec<_>>();

    Ok((to_remove, already_scanned, needs_update))
}

#[inline]
fn get_repo_key_name(repo: &(String, String), arch: &str) -> String {
    if repo.1 == "main" {
        arch.to_string()
    } else {
        format!("{}-{}", repo.1, arch)
    }
}

fn collect_changed_repos(packages: &[PackageMeta]) -> HashMap<String, RepositoryMeta> {
    let mut repos = HashMap::new();
    for p in packages {
        let key = get_repo_key_name(&p.repo, &p.deb.arch);
        let path = format!("{}/{}", p.repo.0, p.repo.1);
        let name = format!("{}/{}", key, p.repo.0);
        repos.insert(
            name.clone(),
            RepositoryMeta {
                name,
                key,
                path,
                branch: p.repo.0.clone(),
                component: p.repo.1.clone(),
                architecture: p.deb.arch.clone(),
            },
        );
    }

    repos
}

pub async fn update_unchanged_packages(
    pool: &PgPool,
    packages: SegQueue<(PathBuf, u64)>,
    mirror_root: &Path,
) -> Result<()> {
    while let Some(package) = packages.pop() {
        if let Ok(path) = package.0.strip_prefix(mirror_root) {
            info!("Updating {} ...", path.display());
            if let Some(path) = path.to_str() {
                sqlx::query!(
                    "UPDATE pv_packages SET mtime = $1 WHERE filename = $2",
                    package.1 as i64,
                    path
                )
                .execute(pool)
                .await?;
            } else {
                warn!("{} contains invalid characters!", path.display());
            }
        } else {
            warn!(
                "{} is not in the package pool directory!",
                package.0.display()
            );
        }
    }

    Ok(())
}

/// Get what and how packages changed (needs to be run before `save_packages_to_db`)
pub async fn what_changed(pool: &PgPool, packages: &[PackageMeta]) -> Result<Vec<PVMessage>> {
    let mut messages = Vec::with_capacity(packages.len());
    for p in packages {
        let key = get_repo_key_name(&p.repo, &p.deb.arch);
        let repo = format!("{}/{}", key, p.repo.0);
        let record = sqlx::query!(
            r#"SELECT comparable_dpkgver($1) > _vercomp AS newer, version, filename FROM pv_packages 
WHERE package=$2 AND repo=$3 AND _vercomp=
(SELECT max("_vercomp") FROM pv_packages WHERE package=$2 AND repo=$3 GROUP BY package)"#,
            p.deb.version,
            p.deb.name,
            repo
        )
        .fetch_optional(pool)
        .await?;
        // not found: new package
        if record.is_none() {
            messages.push(PVMessage::new(
                format!("{}-{}", p.repo.0, p.repo.1),
                p.deb.name.clone(),
                p.deb.arch.clone(),
                b'+',
                None,
                Some(p.deb.version.clone()),
            ));
            continue;
        }
        let record = record.unwrap();
        let method = if record.newer.unwrap_or(false) {
            b'^'
        } else if p.deb.version == record.version {
            b'*'
        } else {
            // not a new package, version is not newer: older package
            continue;
        };
        messages.push(PVMessage::new(
            format!("{}-{}", p.repo.0, p.repo.1),
            p.deb.name.clone(),
            p.deb.arch.clone(),
            method,
            Some(record.version),
            Some(p.deb.version.clone()),
        ));
    }

    Ok(messages)
}

pub async fn update_changed_repos(pool: &PgPool, packages: &[PackageMeta]) -> Result<()> {
    let changed_repos = collect_changed_repos(packages);
    let mut tx = pool.begin().await?;
    for (_, repo) in changed_repos {
        sqlx::query!(
            "INSERT INTO pv_repos VALUES ($1, $2, $3, $4, $5, $6, now())
ON CONFLICT (name) DO UPDATE SET mtime=now()",
            repo.name,
            repo.path,
            if repo.branch == "stable" { 0 } else { 1 },
            repo.branch,
            repo.component,
            repo.architecture
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(())
}

pub async fn save_packages_to_db(pool: &PgPool, packages: &[PackageMeta]) -> Result<()> {
    let mut tx = pool.begin().await?;
    for pkg in packages {
        save_package_to_db(&mut tx, pkg).await?;
    }
    tx.commit().await?;

    Ok(())
}

#[inline]
fn split_so_name(name: &str) -> (Option<&str>, Option<&str>) {
    let splitter = name.find(".so");
    let so_name = splitter.map(|s| &name[..(s + 3)]);
    let so_version = splitter.and_then(|s| {
        if s >= (name.len() - 3) {
            None
        } else {
            Some(&name[(s + 3)..])
        }
    });

    (so_name, so_version)
}

#[inline]
fn normalize_path(path: &str) -> &str {
    // . -> <EMPTY>
    if path == "." {
        return "";
    }

    // ./usr -> usr
    // /usr -> usr
    path.strip_prefix("./")
        .unwrap_or_else(|| path.strip_prefix('/').unwrap_or(path))
}

async fn save_package_to_db(
    pool: &mut Transaction<'_, Postgres>,
    package: &PackageMeta,
) -> Result<()> {
    let meta = &package.deb;
    let contents = &package.contents;
    let repo = format!(
        "{}/{}",
        get_repo_key_name(&package.repo, &meta.arch),
        package.repo.0
    );
    let result = sqlx::query!(
        r#"INSERT INTO pv_packages VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, comparable_dpkgver($2), $14)
ON CONFLICT (package, version, repo)
DO UPDATE SET filename=$5,size=$6,sha256=$7,mtime=$8,debtime=$9,section=$10,installed_size=$11,maintainer=$12,description=$13,features=$14
RETURNING (xmax = 0) AS new"#,
        meta.name, meta.version, repo, meta.arch, package.filename, package.size as i64, package.sha256, package.mtime as i32, meta.debtime as i32, meta.section, meta.inst_size.parse::<i64>().unwrap_or(0),
        meta.maintainer, meta.desc, meta.features,
    ).fetch_one(&mut **pool).await?;
    if !result.new.unwrap_or(false) {
        warn!("{} is a duplicate!", package.filename);
        // remove duplicated data and append this package to duplicate list
        sqlx::query!(
            r#"WITH d1 AS (DELETE FROM pv_package_sodep WHERE package=$1 AND version=$2 AND repo=$3 RETURNING package)
, d2 AS (DELETE FROM pv_package_files WHERE package=$1 AND version=$2 AND repo=$3 RETURNING package)
, d3 AS (DELETE FROM pv_package_dependencies WHERE package=$1 AND version=$2 AND repo=$3 RETURNING package)
DELETE FROM pv_package_duplicate WHERE package=$1 AND version=$2 AND repo=$3"#,
            meta.name,
            meta.version,
            repo
        ).execute(&mut **pool).await?;
        sqlx::query!(
            "INSERT INTO pv_package_duplicate SELECT * FROM pv_packages WHERE filename=$1",
            package.filename
        )
        .execute(&mut **pool)
        .await?;
    }
    // update dependencies information
    for dep in PKG_RELATION {
        if let Some(d) = meta.extra.get(dep.as_bytes()) {
            let value =
                std::str::from_utf8(d)
                    .ok()
                    .and_then(|x| if x.is_empty() { None } else { Some(x) });
            if let Some(value) = value {
                sqlx::query!(
                    "INSERT INTO pv_package_dependencies VALUES($1, $2, $3, $4, $5) ON CONFLICT ON CONSTRAINT pv_package_dependencies_pkey DO UPDATE SET value = $5",
                    meta.name,
                    meta.version,
                    repo,
                    dep,
                    value
                )
                .execute(&mut **pool)
                .await?;
            }
        }
    }
    // update so information
    for so in &contents.so_requires {
        let (so_name, so_version) = split_so_name(so);
        sqlx::query!(
            "INSERT INTO pv_package_sodep VALUES ($1, $2, $3, 1, $4, $5)",
            meta.name,
            meta.version,
            repo,
            so_name,
            so_version
        )
        .execute(&mut **pool)
        .await?;
    }
    for so in &contents.so_provides {
        let (so_name, so_version) = split_so_name(so);
        sqlx::query!(
            "INSERT INTO pv_package_sodep VALUES ($1, $2, $3, 0, $4, $5)",
            meta.name,
            meta.version,
            repo,
            so_name,
            so_version
        )
        .execute(&mut **pool)
        .await?;
    }
    // update files information
    for f in &contents.files {
        let path = f.path.parent().and_then(|p| p.to_str()).map(normalize_path);
        let filename = f.path.file_name().and_then(|p| p.to_str());
        let uname = f.uname.as_ref().and_then(|p| std::str::from_utf8(p).ok());
        let gname = f.gname.as_ref().and_then(|p| std::str::from_utf8(p).ok());
        sqlx::query!(
            r#"INSERT INTO pv_package_files VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
            meta.name, meta.version, repo, path, filename, f.size as i64, f.type_ as i16, f.perms as i32, f.uid as i64, f.gid as i64, uname, gname
        ).execute(&mut **pool).await?;
    }

    Ok(())
}

fn get_branch_name<P: AsRef<Path>>(rel_path: P) -> Result<(String, String)> {
    let mut comp = rel_path.as_ref().strip_prefix("pool")?.components();
    let mut branch = None;
    for _ in 0..=1 {
        let cur = match comp.next() {
            Some(Component::Normal(p)) => p.to_string_lossy(),
            Some(_) | None => {
                return Err(anyhow!(
                    "Unexpected path component: {}",
                    rel_path.as_ref().display()
                ))
            }
        };
        if let Some(branch) = branch {
            return Ok((branch, cur.to_string()));
        } else {
            branch = Some(cur.to_string());
        }
    }

    Err(anyhow!(
        "Unable to determine branch name for {}",
        rel_path.as_ref().display()
    ))
}

#[inline]
fn is_shared_object(path: &[u8]) -> bool {
    // condition: inside `/usr/lib` or `/lib` with a name ends with `.so` or contains `.so.`
    (path.starts_with(b"./usr/lib/") || path.starts_with(b"./lib/"))
        && (path.ends_with(b".so")
            || std::str::from_utf8(path)
                .unwrap_or_default()
                .contains(".so."))
}

fn parse_elf(bytes: &[u8]) -> Result<Vec<&str>> {
    use goblin::{
        container::{Container, Ctx, Endian},
        elf::{Dynamic, Elf, ProgramHeader},
        strtab::Strtab,
    };
    let mut libraries = Vec::new();
    let header = Elf::parse_header(bytes)?;
    let elf = Elf::lazy_parse(header)?;
    let container = if elf.is_64 {
        Container::Big
    } else {
        Container::Little
    };
    let ctx = Ctx::new(
        container,
        if elf.little_endian {
            Endian::Little
        } else {
            Endian::Big
        },
    );
    let prog_headers =
        ProgramHeader::parse(bytes, header.e_phoff as usize, header.e_phnum as usize, ctx)?;
    let dynamic = Dynamic::parse(bytes, &prog_headers, ctx)?;
    if let Some(ref dynamic) = dynamic {
        let dyn_info = &dynamic.info;
        let dynstrtab = Strtab::parse(bytes, dyn_info.strtab, dyn_info.strsz, 0x0)?;
        if dyn_info.needed_count > 0 {
            libraries = dynamic.get_libraries(&dynstrtab);
        }
    }

    Ok(libraries)
}

/// Scan ELF files for required libraries and soname information
fn scan_elf<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    provides: &mut HashSet<String>,
    requires: &mut HashSet<String>,
) -> Result<()> {
    let header = entry.header();
    // check if needs to parse as ELF
    if !header.entry_type().is_file() || entry.size() < 4 {
        // not an ELF
        return Ok(());
    }
    let mut elf_header = vec![0u8; 4];
    entry.read_exact(&mut elf_header)?;
    if elf_header != ELF_MAGIC {
        // not an ELF due to invalid magic
        return Ok(());
    }

    let mut content = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut content)?;
    elf_header.extend(content);
    let libraries = parse_elf(&elf_header)?;
    for i in libraries {
        requires.insert(i.to_string());
    }

    // we should not use SONAME for so provides, since the dynamic linker only
    // uses the file name to handle DT_NEEDED requests
    if is_shared_object(&entry.path_bytes()) {
        let path = entry.path();
        if let Ok(path) = path {
            if let Some(f) = path.file_name() {
                provides.insert(f.to_string_lossy().to_string());
            }
        }
    }

    Ok(())
}

/// Collect information on the package file contents
fn collect_files<R: Read>(reader: R) -> Result<PackageContents> {
    let mut provides = HashSet::new();
    let mut requires = HashSet::new();
    let mut tar = TarArchive::new(reader);
    let mut files = Vec::with_capacity(100);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let header = entry.header();
        files.push(PackageFile {
            path: entry.path()?.to_path_buf(),
            is_dir: header.entry_type().is_dir(),
            size: entry.size(),
            type_: header.entry_type().as_byte(),
            perms: header.mode()?,
            uid: header.uid().unwrap_or(0),
            gid: header.gid().unwrap_or(0),
            uname: header.username_bytes().map(|x| x.to_owned()),
            gname: header.groupname_bytes().map(|x| x.to_owned()),
        });
        // ================= ELF processing
        //
        // Find so provides and requires
        //
        // ELF .so provides are collected from file names instead of SONAME, because
        // there are cases when the shared library has no SONAME e.g. libardourcp.so
        // and libautofs.so. Some shared libraries deliberately uses a SONAME that
        // differs from its name, e.g. CUDA's stub libcuda.so has SONAME libcuda.so.1,
        // which resides in the NVIDIA driver.
        //
        // ELF .so requires are collected from DT_NEEDED of ELF files. In the future,
        // we may want to handle dlopen-ed libraries.
        //
        // For symlinks, we skip the expensive symlink resolving process and check
        // whether it is a shared library by its name. Otherwise, we will also verify
        // that it is in ELF format.
        if is_shared_object(&entry.path_bytes()) && header.entry_type().is_symlink() {
            let path = entry.path();
            if let Ok(path) = path {
                if let Some(f) = path.file_name() {
                    provides.insert(f.to_string_lossy().to_string());
                }
            }
        }
        if let Err(e) = scan_elf(&mut entry, &mut provides, &mut requires) {
            let file_path = entry.path()?.to_path_buf();
            error!(
                "Problems parsing ELF: {:?}",
                e.context(format!("when checking {:?}", file_path))
            );
        }
    }

    Ok(PackageContents {
        files,
        so_provides: provides,
        so_requires: requires,
    })
}

/// Advanced deb package reader. Scans control and package files
fn open_deb_advanced<'a, R: Read + 'a>(
    reader: HashedReader<R>,
    stat: Metadata,
    filename: &str,
    branch: (String, String),
) -> Result<PackageMeta> {
    let mut deb = ArArchive::new(reader);
    let mut metadata = None;
    let mut files = None;
    while let Some(entry) = deb.next_entry() {
        if entry.is_err() {
            continue;
        }
        let entry = entry?;
        let filename = entry.header().identifier();
        if filename.starts_with(b"control.tar") {
            let debtime = entry.header().mtime();
            let format = determine_format(filename)?;
            let control = open_compressed_control(entry, &format)?;
            let meta = crate::parser::single_package_map(&control);
            if let Err(e) = meta {
                return Err(anyhow!("{:?}", e));
            }
            let parsed_control = meta.unwrap();
            let mut meta = parsed_control.1;
            metadata = Some(DebMeta {
                name: must_have!(meta, "Package"),
                version: must_have!(meta, "Version"),
                section: must_have!(meta, "Section"),
                desc: must_have!(meta, "Description"),
                arch: must_have!(meta, "Architecture"),
                inst_size: must_have!(meta, "Installed-Size"),
                maintainer: must_have!(meta, "Maintainer"),
                features: meta
                    .remove("X-AOSC-Features".as_bytes())
                    .map(|x| String::from_utf8_lossy(&x).to_string()),
                extra: collect_left_over_fields(meta),
                debtime,
            });
        } else if filename.starts_with(b"data.tar") {
            let format = determine_format(filename)?;
            files = Some(open_compressed_data(entry, &format)?);
        }
    }

    if metadata.is_none() || files.is_none() {
        Err(anyhow!("data archive not found or format unsupported"))
    } else {
        let sha256 = deb.into_inner()?.get_hash()?;
        let metadata = metadata.unwrap();
        let mtime = mtime(&stat)?;
        Ok(PackageMeta {
            repo: branch,
            deb: metadata,
            size: stat.len(),
            filename: filename.to_string(),
            sha256,
            mtime,
            contents: files.unwrap(),
        })
    }
}

/// Advanced version of scanning deb packages. With bells and whistles.
pub(crate) fn scan_single_deb_advanced<P: AsRef<Path>>(path: P, root: P) -> Result<PackageMeta> {
    let stat = path.as_ref().metadata()?;
    let f = File::open(path.as_ref())?;
    let f = unsafe { memmap2::Mmap::map(&f)? };
    let rel_filename = path.as_ref().strip_prefix(root.as_ref())?;
    let component = get_branch_name(rel_filename)?;

    open_deb_advanced(
        HashedReader::new(&*f),
        stat,
        &rel_filename.to_string_lossy(),
        component,
    )
}

#[test]
fn test_deb_adv() {
    let content = scan_single_deb_advanced(
        "./tests/pool/tests/fixtures/a2jmidid_9-0_amd64.deb",
        "./tests",
    )
    .unwrap();
    assert_eq!(
        &content.sha256,
        "6a7dd466854f6c1f4a597f0c547acf1f90d8298a04f4a2ca31f96a7c9dca8bc3"
    );
    println!("{:?}", content);

    let content = scan_single_deb_advanced(
        "./tests/pool/tests/fixtures/aosc-aaa_11.6.0-1~pre20241017T062346Z_amd64.deb",
        "./tests",
    )
    .unwrap();
    assert_eq!(content.deb.features, Some("core".to_string()));

    println!("{:?}", content);
}

#[test]
fn so_name_splitter() {
    let so = "libclang.so.1";
    assert_eq!(split_so_name(so), (Some("libclang.so"), Some(".1")));
    let so = "libclang.so";
    assert_eq!(split_so_name(so), (Some("libclang.so"), None));
}
