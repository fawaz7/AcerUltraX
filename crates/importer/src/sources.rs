//! The files the importer can see: folders, zip downloads, and the Windows app packages inside them
//! (`.appxbundle`/`.msixbundle` holding `.appx`/`.msix`, which are zip files too).

use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use zip::ZipArchive;

/// A file somewhere under the sources, with its path split into components. Zip files made on Windows may use
/// backslashes, which Linux unzip tools keep inside file names, so those count as separators too.
pub struct Entry {
    pub parts: Vec<String>,
    origin: Origin,
}

enum Origin {
    File(PathBuf),
    Zip { archive: Arc<Archive>, index: usize },
}

/// A zip archive: a file on disk, a stored (uncompressed) member of another archive, or bytes in memory.
enum Archive {
    File(PathBuf),
    Slice { file: PathBuf, start: u64, len: u64 },
    Mem(Arc<Vec<u8>>),
}

/// A window onto part of a file, so an uncompressed archive inside an archive is read in place.
struct SubFile {
    f: File,
    start: u64,
    len: u64,
    pos: u64,
}

impl Read for SubFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.len.saturating_sub(self.pos) as usize;
        let n = buf.len().min(left);
        self.f.seek(SeekFrom::Start(self.start + self.pos))?;
        let n = self.f.read(&mut buf[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SubFile {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let p = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.len as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if p < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

/// Shared archive bytes (opening an archive again does not copy them).
struct Bytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

impl Archive {
    fn open(&self) -> std::io::Result<ZipArchive<Box<dyn ReadSeek>>> {
        let r: Box<dyn ReadSeek> = match self {
            Archive::File(p) => Box::new(File::open(p)?),
            Archive::Slice { file, start, len } => {
                Box::new(SubFile { f: File::open(file)?, start: *start, len: *len, pos: 0 })
            }
            Archive::Mem(b) => Box::new(Cursor::new(Bytes(b.clone()))),
        };
        ZipArchive::new(r).map_err(std::io::Error::other)
    }

    /// The disk file and offset of this archive's own bytes, for reading stored members in place.
    fn on_disk(&self) -> Option<(&Path, u64)> {
        match self {
            Archive::File(p) => Some((p, 0)),
            Archive::Slice { file, start, .. } => Some((file, *start)),
            Archive::Mem(_) => None,
        }
    }
}

impl Entry {
    pub fn name(&self) -> &str {
        self.parts.last().map_or("", |s| s.as_str())
    }

    pub fn parent(&self) -> &[String] {
        &self.parts[..self.parts.len().saturating_sub(1)]
    }

    pub fn read(&self) -> std::io::Result<Vec<u8>> {
        match &self.origin {
            Origin::File(p) => std::fs::read(p),
            Origin::Zip { archive, index } => {
                let mut z = archive.open()?;
                let mut f = z.by_index(*index).map_err(std::io::Error::other)?;
                let mut v = Vec::with_capacity(f.size() as usize);
                f.read_to_end(&mut v)?;
                Ok(v)
            }
        }
    }

    /// Where it came from, for messages.
    pub fn describe(&self) -> String {
        self.parts.join("/")
    }
}

fn split(name: &str) -> impl Iterator<Item = String> + '_ {
    name.split(['/', '\\']).filter(|s| !s.is_empty()).map(String::from)
}

fn is_archive(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".zip", ".appx", ".msix", ".appxbundle", ".msixbundle"].iter().any(|e| n.ends_with(e))
}

/// App-package parts that never hold anything we need (translations, icons, other CPUs).
fn skip_package(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["language-", "scale-", "_arm", "_x86", "microsoft.", "vclibs", ".net.native"].iter().any(|s| n.contains(s))
}

/// Every file under `paths` (folders and archives, recursively).
pub fn collect(paths: &[PathBuf]) -> Vec<Entry> {
    let mut out = Vec::new();
    for p in paths {
        let base = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if p.is_dir() {
            walk_dir(p, &[base], &mut out);
        } else if is_archive(&base) {
            walk_archive(Arc::new(Archive::File(p.clone())), &[base], &mut out);
        } else if p.is_file() {
            out.push(Entry { parts: split(&base).collect(), origin: Origin::File(p.clone()) });
        }
    }
    out
}

fn walk_dir(dir: &Path, prefix: &[String], out: &mut Vec<Entry>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let mut parts = prefix.to_vec();
        parts.extend(split(&name));
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() => walk_dir(&p, &parts, out),
            Ok(m) if m.is_file() && is_archive(&name) && !skip_package(&name) => {
                walk_archive(Arc::new(Archive::File(p)), &parts, out)
            }
            Ok(m) if m.is_file() => out.push(Entry { parts, origin: Origin::File(p) }),
            _ => {}
        }
    }
}

fn walk_archive(archive: Arc<Archive>, prefix: &[String], out: &mut Vec<Entry>) {
    let Ok(mut z) = archive.open() else {
        log::warn!("cannot read archive {}", prefix.join("/"));
        return;
    };
    for index in 0..z.len() {
        let Ok(f) = z.by_index(index) else { continue };
        if f.is_dir() {
            continue;
        }
        let name = f.name().map(|n| n.into_owned()).unwrap_or_default();
        let mut parts = prefix.to_vec();
        parts.extend(split(&name));
        if is_archive(&name) {
            if skip_package(&name) {
                continue;
            }
            // Read a stored (uncompressed) inner archive in place; inflate a compressed one into memory.
            let inner = match (archive.on_disk(), f.compression(), f.data_start()) {
                (Some((file, base)), zip::CompressionMethod::Stored, Some(start)) => {
                    Some(Archive::Slice { file: file.to_path_buf(), start: base + start, len: f.size() })
                }
                _ => {
                    drop(f);
                    let mut v = Vec::new();
                    z.by_index(index)
                        .ok()
                        .and_then(|mut f| f.read_to_end(&mut v).ok())
                        .map(|_| Archive::Mem(Arc::new(v)))
                }
            };
            if let Some(a) = inner {
                walk_archive(Arc::new(a), &parts, out);
            }
            continue;
        }
        out.push(Entry { parts, origin: Origin::Zip { archive: archive.clone(), index } });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_bytes(files: &[(&str, &[u8])], stored: bool) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let method = if stored { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
        let opts = zip::write::SimpleFileOptions::default().compression_method(method);
        for (n, d) in files {
            w.start_file(*n, opts).unwrap();
            w.write_all(d).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn nested_archives() {
        let msix = zip_bytes(&[("Data/HeadphoneTuning/x.db", b"db"), ("Assets/logo.png", b"png")], false);
        let bundle = zip_bytes(&[("App_x64.msix", &msix), ("App_language-de.msix", &msix)], true);
        let outer = zip_bytes(&[("Utility\\pkg.appxbundle", &bundle), ("readme.txt", b"hi")], false);
        let dir = std::env::temp_dir().join(format!("acerultrax-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("download.zip");
        std::fs::write(&p, &outer).unwrap();
        let entries = collect(&[p]);
        let db = entries.iter().find(|e| e.name() == "x.db").expect("found inside zip > bundle > msix");
        assert_eq!(db.read().unwrap(), b"db");
        assert_eq!(db.parts[..3], ["download.zip", "Utility", "pkg.appxbundle"]);
        assert_eq!(entries.iter().filter(|e| e.name() == "x.db").count(), 1, "language packages are skipped");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
