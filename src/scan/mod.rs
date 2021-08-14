use anyhow::{anyhow, Result};
use ar::Archive as ArArchive;
use faster_hex::hex_string;
use flate2::read::GzDecoder;
use log::{error, info};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::fs::Metadata;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use std::{io::Read, path::Path};
use tar::Archive as TarArchive;
use walkdir::{DirEntry, WalkDir};
use xz2::read::XzDecoder;

mod dbscan;

pub use self::dbscan::*;

#[macro_export]
macro_rules! read_compressed {
    ($format:ident, $func:ident [ $reader:ident ]) => {{
        match $format {
            TarFormat::Xzip => $func(XzDecoder::new($reader)),
            TarFormat::Gzip => $func(GzDecoder::new($reader)),
        }
    }};
}

pub(crate) fn mtime(stat: &Metadata) -> Result<u64> {
    Ok(stat.modified()?.duration_since(UNIX_EPOCH)?.as_secs())
}

enum TarFormat {
    Xzip,
    Gzip,
}

/// Collect control information
fn collect_control<R: Read>(reader: R) -> Result<Vec<u8>> {
    let mut tar = TarArchive::new(reader);
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.path_bytes().as_ref() == &b"./control"[..] {
            let mut buf = Vec::new();
            buf.reserve(1024);
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }

    Err(anyhow!("Could not read control file"))
}

fn open_compressed_control<R: Read>(reader: R, format: &TarFormat) -> Result<Vec<u8>> {
    read_compressed!(format, collect_control[reader])
}

/// Determine the compression format based on the extension name
fn determine_format(format: &[u8]) -> Result<TarFormat> {
    if format.ends_with(b".xz") {
        Ok(TarFormat::Xzip)
    } else if format.ends_with(b".gz") {
        Ok(TarFormat::Gzip)
    } else {
        Err(anyhow!("Unknown format: {:?}", format))
    }
}

/// Calculate the Sha256 checksum of the given stream
pub fn sha256sum<R: Read>(mut reader: R) -> Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader, &mut hasher)?;

    Ok(hex_string(&hasher.finalize()))
}

#[inline]
fn is_deb(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.ends_with(".deb"))
        .unwrap_or(false)
}

pub fn scan_packages_advanced(entries: &[&Path], root: &Path) -> Vec<PackageMeta> {
    entries
        .par_iter()
        .filter_map(|entry| {
            info!("Scanning {} ...", entry.display());
            match scan_single_deb_advanced(entry, &root) {
                Ok(meta) => Some(meta),
                Err(err) => {
                    error!("{}: {:?}", entry.display(), err);
                    None
                }
            }
        })
        .collect()
}

/// Auto-discover topics and components under the specified directory
pub fn discover_topics_components<P: AsRef<Path>>(path: P) -> Result<Vec<PathBuf>> {
    let mut topics = Vec::new();

    for entry in WalkDir::new(path.as_ref())
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_entry(|x| x.file_type().is_dir())
    {
        let entry = entry?;
        let name = entry.path().strip_prefix(path.as_ref())?;
        topics.push(name.to_owned());
    }

    Ok(topics)
}

/// Walk through all the packages in a repository (no scanning)
pub fn collect_all_packages<P: AsRef<Path>>(path: P) -> Result<Vec<DirEntry>> {
    let mut files = Vec::new();
    files.reserve(1000);
    for entry in WalkDir::new(path.as_ref()) {
        let entry = entry?;
        if is_deb(&entry) {
            files.push(entry);
        }
    }

    Ok(files)
}
