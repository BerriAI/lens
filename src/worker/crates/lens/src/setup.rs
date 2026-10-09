use crate::Error;
use rand::RngCore;
use std::{io::Write, path::Path};

pub fn initialize(directory: &Path) -> Result<bool, Error> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(".env");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => return Ok(false),
        Ok(_) => return Err(Error::Configuration(".env must be a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    writeln!(file, "LENS_ADMIN_TOKEN={}", secret())?;
    writeln!(file, "CLICKHOUSE_PASSWORD={}", secret())?;
    writeln!(file, "LENS_PUBLIC_URL=http://localhost:4318")?;
    file.as_file().sync_all()?;
    match file.persist_noclobber(&path) {
        Ok(_) => Ok(true),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::symlink_metadata(path)?.is_file() {
                Ok(false)
            } else {
                Err(Error::Configuration(".env must be a regular file"))
            }
        }
        Err(error) => Err(error.error.into()),
    }
}

fn secret() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};

    #[fixture]
    fn directory() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[rstest]
    fn fresh_setup_generates_distinct_private_credentials(directory: tempfile::TempDir) {
        assert!(initialize(directory.path()).unwrap());
        let path = directory.path().join(".env");
        let content = std::fs::read_to_string(&path).unwrap();
        let settings: std::collections::BTreeMap<_, _> = content
            .lines()
            .map(|line| line.split_once('=').unwrap())
            .collect();
        let admin = settings["LENS_ADMIN_TOKEN"];
        let storage = settings["CLICKHOUSE_PASSWORD"];
        assert_eq!(admin.len(), 64);
        assert_eq!(storage.len(), 64);
        assert_ne!(admin, storage);
        assert_eq!(settings["LENS_PUBLIC_URL"], "http://localhost:4318");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[rstest]
    fn repeated_setup_keeps_credentials_and_custom_settings(directory: tempfile::TempDir) {
        assert!(initialize(directory.path()).unwrap());
        let path = directory.path().join(".env");
        let original = format!(
            "{}CUSTOM_SETTING=kept\n",
            std::fs::read_to_string(&path).unwrap()
        );
        std::fs::write(&path, &original).unwrap();
        assert!(!initialize(directory.path()).unwrap());
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }

    #[rstest]
    fn simultaneous_setup_has_one_winner(directory: tempfile::TempDir) {
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| initialize(directory.path()).unwrap());
            let second = scope.spawn(|| initialize(directory.path()).unwrap());
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_ne!(results.0, results.1);
        assert_eq!(
            std::fs::read_to_string(directory.path().join(".env"))
                .unwrap()
                .lines()
                .count(),
            3
        );
    }

    #[cfg(unix)]
    #[rstest]
    fn setup_rejects_symlinks_without_modifying_target(directory: tempfile::TempDir) {
        let target = directory.path().join("private");
        std::fs::write(&target, "retained").unwrap();
        std::os::unix::fs::symlink(&target, directory.path().join(".env")).unwrap();
        assert!(initialize(directory.path()).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "retained");
    }
}
