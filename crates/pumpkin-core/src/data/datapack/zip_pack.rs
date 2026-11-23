//! Unpacks zip data packs into a cache folder so they load like folder packs.
//!
//! The cache lives in the world's `datapacks` directory, and the unpacked folder is the
//! pack's `LoadedDatapack::root_path`, so later structure lookups read from it too.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Folder under `datapacks` that holds unpacked zip packs. The leading dot keeps
/// the pack scanner from treating it as a pack.
const ZIP_CACHE_DIR: &str = ".zip_cache";
const MAX_ZIP_PACK_ENTRIES: usize = 65_536;
const MAX_ZIP_PACK_BYTES: u64 = 1 << 30;

/// Returns the folder holding the contents of `zip_path`, unpacking it first
/// when there is no cached copy or the zip changed since it was unpacked.
pub fn extracted_zip_pack(
    datapacks_dir: &Path,
    zip_path: &Path,
    file_name: &str,
) -> Result<PathBuf, String> {
    let cache_dir = datapacks_dir.join(ZIP_CACHE_DIR);
    let pack_dir = cache_dir.join(file_name);
    let stamp_path = cache_dir.join(format!("{file_name}.source"));
    let stamp = zip_stamp(zip_path)?;

    if pack_dir.is_dir()
        && let Some(stamp) = &stamp
        && fs::read_to_string(&stamp_path).is_ok_and(|cached| &cached == stamp)
    {
        return Ok(pack_dir);
    }

    let partial_dir = cache_dir.join(format!("{file_name}.partial"));
    remove_dir_if_present(&partial_dir)?;
    if let Err(error) = unpack_zip(zip_path, &partial_dir) {
        remove_dir_if_present(&partial_dir)?;
        return Err(error);
    }
    remove_dir_if_present(&pack_dir)?;
    fs::rename(&partial_dir, &pack_dir).map_err(|error| error.to_string())?;
    if let Some(stamp) = stamp {
        fs::write(&stamp_path, stamp).map_err(|error| error.to_string())?;
    }
    Ok(pack_dir)
}

/// Size and modification time of the zip, or `None` when the file system has no modification time.
fn zip_stamp(zip_path: &Path) -> Result<Option<String>, String> {
    let metadata = fs::metadata(zip_path).map_err(|error| error.to_string())?;
    Ok(metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|modified| format!("{}:{}", metadata.len(), modified.as_nanos())))
}

fn unpack_zip(zip_path: &Path, target: &Path) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    if archive.len() > MAX_ZIP_PACK_ENTRIES {
        return Err(format!("More than {MAX_ZIP_PACK_ENTRIES} entries"));
    }

    fs::create_dir_all(target).map_err(|error| error.to_string())?;
    let mut remaining = MAX_ZIP_PACK_BYTES;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        // Entries that would land outside the pack (absolute paths, `..`) can't be
        // resources of the pack, so they are skipped.
        let Some(relative_path) = entry.enclosed_name() else {
            continue;
        };
        let out_path = target.join(relative_path);
        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let mut out = File::create(&out_path).map_err(|error| error.to_string())?;
        // The sizes in the zip's headers can't be trusted, so count the bytes actually written.
        let copied = io::copy(&mut entry.by_ref().take(remaining + 1), &mut out)
            .map_err(|error| error.to_string())?;
        if copied > remaining {
            return Err(format!("Unpacks to more than {MAX_ZIP_PACK_BYTES} bytes"));
        }
        remaining -= copied;
    }
    Ok(())
}

fn remove_dir_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;

    use zip::write::SimpleFileOptions;

    use super::extracted_zip_pack;

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut writer = zip::ZipWriter::new(File::create(path).expect("create zip"));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, content) in entries {
            writer.start_file(*name, options).expect("start zip entry");
            writer
                .write_all(content.as_bytes())
                .expect("write zip entry");
        }
        writer.finish().expect("finish zip");
    }

    #[test]
    fn unpacks_zip_packs_and_refreshes_changed_zips() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let datapacks_dir = temp_dir.path().join("datapacks");
        fs::create_dir_all(&datapacks_dir).expect("datapacks dir");
        let zip_path = datapacks_dir.join("pack.zip");

        write_zip(
            &zip_path,
            &[
                ("pack.mcmeta", "{}"),
                ("data/test/function/hi.mcfunction", "say one"),
                ("../outside.txt", "escaped"),
            ],
        );
        let pack_dir =
            extracted_zip_pack(&datapacks_dir, &zip_path, "pack.zip").expect("unpack zip");
        let function_path = pack_dir.join("data/test/function/hi.mcfunction");
        assert_eq!(
            fs::read_to_string(&function_path).expect("read function"),
            "say one"
        );
        assert!(pack_dir.join("pack.mcmeta").is_file());
        assert!(!datapacks_dir.join(".zip_cache/outside.txt").exists());

        write_zip(
            &zip_path,
            &[
                ("pack.mcmeta", "{}"),
                ("data/test/function/hi.mcfunction", "say two, longer"),
            ],
        );
        let pack_dir =
            extracted_zip_pack(&datapacks_dir, &zip_path, "pack.zip").expect("unpack zip again");
        assert_eq!(
            fs::read_to_string(pack_dir.join("data/test/function/hi.mcfunction"))
                .expect("read function"),
            "say two, longer"
        );
    }
}
