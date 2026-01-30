pub mod fmt {
  use std::fmt;

  pub struct Size(pub u64);

  impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let i = (self.0 | 1).ilog2() as usize / 10;
      let size = self.0 as f64 / (1u64 << 10 * i) as f64;
      let prec = i.min(1) * 2;
      let unit = ["", "Ki", "Mi", "Gi", "Ti", "Pi", "Ei"][i];
      write!(f, "{size:.prec$} {unit}B")
    }
  }

  pub fn percentage(n: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { n as f64 / total as f64 * 100.0 }
  }
}

pub mod fs {
  use std::fs::{DirEntry, File};
  use std::hash::{DefaultHasher, Hasher};
  use std::path::{Path, PathBuf};
  use std::{fs, io, io::prelude::*};

  pub fn hash(path: &Path) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut hasher = DefaultHasher::new();
    let mut buffer = [0; 1 << 16];
    loop {
      match file.read(&mut buffer)? {
        0 => break Ok(hasher.finish()),
        n => hasher.write(&buffer[..n]),
      }
    }
  }

  pub fn scan<F>(cwd: &Path, f: &mut F) -> io::Result<()>
  where
    F: FnMut(&Path, io::Result<(PathBuf, DirEntry)>) -> io::Result<()>,
  {
    let entries = match fs::read_dir(cwd) {
      Err(err) => return f(cwd, Err(err)),
      Ok(entries) => entries,
    };

    for e in entries {
      match e.and_then(|e| Ok((e.file_type()?, e.path(), e))) {
        Err(err) => f(cwd, Err(err))?,
        Ok((file_type, path, entry)) => match file_type {
          ft if ft.is_dir() => scan(&path, f)?,
          ft if ft.is_file() => f(cwd, Ok((path, entry)))?,
          _ => {}
        },
      }
    }

    Ok(())
  }
}
