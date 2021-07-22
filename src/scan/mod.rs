use anyhow::{anyhow, Result};
use ar::Archive as ArArchive;
use faster_hex::hex_string;
use flate2::read::GzDecoder;
use log::{error, info};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::fs::Metadata;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
};
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

/// Simple deb package reader. Only reads control files. Contents are ignored.
fn open_deb_simple<R: Read>(reader: R) -> Result<Vec<u8>> {
    let mut deb = ArArchive::new(reader);
    while let Some(entry) = deb.next_entry() {
        if entry.is_err() {
            continue;
        }
        let entry = entry?;
        let filename = entry.header().identifier();
        if filename.starts_with(b"control.tar") {
            let format = determine_format(filename)?;
            let control = open_compressed_control(entry, &format)?;
            return Ok(control);
        }
    }

    Err(anyhow!("data archive not found or format unsupported"))
}

/// Simple version of scanning deb packages. No bell and whistles, just copying control files.
fn scan_single_deb_simple<P: AsRef<Path>>(path: P, root: P) -> Result<Vec<u8>> {
    let mut f = File::open(path.as_ref())?;
    let sha256 = sha256sum(&mut f)?;
    let actual_size = f.seek(SeekFrom::Current(0))?;
    f.seek(SeekFrom::Start(0))?;
    let mut control = open_deb_simple(f)?;
    control.reserve(128);
    if control.ends_with(&b"\n\n"[..]) {
        control.pop();
    }
    let rel_path = path.as_ref().strip_prefix(root)?;
    control.extend(format!("Size: {}\n", actual_size).as_bytes());
    control.extend(format!("Filename: {}\n", rel_path.to_string_lossy()).as_bytes());
    control.extend(b"SHA256: ");
    control.extend(sha256.as_bytes());
    control.extend(b"\n\n");

    Ok(control)
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

/// Simple version of scanning, for use in local repository or personal repository
pub fn scan_packages_simple(entries: &[DirEntry], root: &Path) -> Vec<u8> {
    entries
        .par_iter()
        .map(|entry| -> Vec<u8> {
            let path = entry.path();
            info!("{:?}", path);
            match scan_single_deb_simple(path, root) {
                Ok(entry) => entry,
                Err(err) => {
                    error!("{:?}", err);
                    Vec::new()
                }
            }
        })
        .flatten()
        .collect()
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

#[test]
fn test_deb() {
    let content =
        scan_single_deb_simple("tests/fixtures/a2jmidid_9-0_amd64.deb", "tests/fixtures").unwrap();
    println!("{}", String::from_utf8(content).unwrap());
}
