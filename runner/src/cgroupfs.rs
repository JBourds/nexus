use std::io;
use std::path::Path;

/// Abstraction over cgroup filesystem operations, enabling unit testing
/// without root access or a real cgroup v2 hierarchy.
pub trait CgroupFs: std::fmt::Debug + Send {
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn write_file(&self, dir: &Path, filename: &str, data: &[u8]) -> io::Result<()>;
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;
}

/// Real implementation that hits the actual filesystem.
#[derive(Debug)]
pub struct RealCgroupFs;

impl CgroupFs for RealCgroupFs {
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir(path)
    }

    fn write_file(&self, dir: &Path, filename: &str, data: &[u8]) -> io::Result<()> {
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join(filename))
            .and_then(|mut f| io::Write::write_all(&mut f, data))
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(path)
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    /// In-memory mock for testing cgroup operations without a real filesystem.
    #[derive(Debug)]
    pub struct MockCgroupFs {
        dirs: RefCell<HashSet<PathBuf>>,
        files: RefCell<HashMap<PathBuf, Vec<u8>>>,
    }

    impl MockCgroupFs {
        pub fn new() -> Self {
            Self {
                dirs: RefCell::new(HashSet::new()),
                files: RefCell::new(HashMap::new()),
            }
        }

        /// Pre-populate a file so `read_to_string` can find it.
        pub fn seed_file(&self, path: impl Into<PathBuf>, content: &str) {
            self.files
                .borrow_mut()
                .insert(path.into(), content.as_bytes().to_vec());
        }

        /// Pre-populate a directory so child `create_dir` calls succeed.
        pub fn seed_dir(&self, path: impl Into<PathBuf>) {
            self.dirs.borrow_mut().insert(path.into());
        }

        /// Check whether a directory was created.
        pub fn dir_exists(&self, path: &Path) -> bool {
            self.dirs.borrow().contains(path)
        }

        /// Read back the last value written to a cgroup control file.
        pub fn read_written(&self, dir: &Path, filename: &str) -> Option<Vec<u8>> {
            self.files.borrow().get(&dir.join(filename)).cloned()
        }
    }

    impl CgroupFs for MockCgroupFs {
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            // Verify parent exists
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && !self.dirs.borrow().contains(parent) {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("parent dir not found: {}", parent.display()),
                    ));
                }
            }
            self.dirs.borrow_mut().insert(path.to_path_buf());
            Ok(())
        }

        fn write_file(&self, dir: &Path, filename: &str, data: &[u8]) -> io::Result<()> {
            if !self.dirs.borrow().contains(dir) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("dir not found: {}", dir.display()),
                ));
            }
            self.files
                .borrow_mut()
                .insert(dir.join(filename), data.to_vec());
            Ok(())
        }

        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.files
                .borrow()
                .get(path)
                .map(|data| String::from_utf8_lossy(data).into_owned())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("file not found: {}", path.display()),
                    )
                })
        }

        fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
            let path = path.to_path_buf();
            self.dirs.borrow_mut().retain(|d| !d.starts_with(&path));
            self.files.borrow_mut().retain(|f, _| !f.starts_with(&path));
            Ok(())
        }
    }
}
