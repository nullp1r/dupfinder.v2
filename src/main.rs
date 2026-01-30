use std::collections::hash_map::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{env, io, io::prelude::*, thread};

use crossbeam_channel as channel;

use self::stdx::{fmt, fs};

mod stdx;

type Count = u64;
type Size = u64;
type Hash = u64;
type FullHash = (Size, Hash);

type Errors = Vec<(PathBuf, io::Error)>;
type Sizes = Vec<(PathBuf, Size)>;
type Hashes = Vec<(PathBuf, FullHash)>;
type SizeCounts = HashMap<Size, Count>;
type HashCounts = HashMap<FullHash, Count>;

fn main() -> io::Result<()> {
  let root = env::args_os().nth(1).unwrap_or_else(|| ".".into());
  let root = Path::new(&root);
  let mut w = io::stdout().lock();
  let (mut errors, mut sizes, mut hashes, mut hash_counts) = Default::default();

  scan(&mut w, &mut errors, &mut sizes, root)?;
  filter(&mut sizes, &mut hashes, &mut hash_counts);

  let t0 = Instant::now();
  compute_hashes(&mut w, sizes, &mut hashes, &mut hash_counts, &mut errors)?;
  let t1 = Instant::now();

  show_duplicate(&mut w, hashes, root)?;
  show_errors(&mut w, errors, root)?;
  show_summary(&mut w, hash_counts, (t1 - t0).as_secs_f64())
}

fn scan(mut w: impl Write, errors: &mut Errors, sizes: &mut Sizes, root: &Path) -> io::Result<()> {
  writeln!(w, "scanning file system…")?;
  write!(w, "files found: \x1b[93m\x1b[s\x1b[?25l")?; // save and hide cursor
  fs::scan(root, &mut |cwd, res| {
    match res.and_then(|(path, file)| Ok((path, file.metadata()?))) {
      Err(err) => errors.push((cwd.into(), err)),
      Ok((path, meta)) => {
        sizes.push((path, meta.len()));
        write!(w, "\x1b[u{}\x1b[K", sizes.len())?;
      }
    };
    Ok(())
  })?;
  writeln!(w, "\x1b[?25h\x1b[39m")?;
  writeln!(w)
}

fn filter(sizes: &mut Sizes, hashes: &mut Hashes, hash_counts: &mut HashCounts) {
  let mut size_counts = SizeCounts::default();
  for &(_, size) in &*sizes {
    *size_counts.entry(size).or_default() += 1;
  }

  let unique = sizes.extract_if(.., |&mut (_, size)| {
    let count = size_counts.get(&size).map_or(0, |&n| n);
    !(size > 0 && count > 1)
  });

  for (path, size) in unique {
    let hash = (size, 0);
    hashes.push((path, hash));
    *hash_counts.entry(hash).or_default() += 1;
  }
}

fn compute_hashes(mut w: impl Write, sizes: Sizes, hashes: &mut Hashes, hash_counts: &mut HashCounts, errors: &mut Errors) -> io::Result<()> {
  let threads = thread::available_parallelism()?.get();
  let (inputs_tx, inputs_rx) = channel::bounded::<(PathBuf, Size)>(threads);
  let (outputs_tx, outputs_rx) = channel::bounded::<(PathBuf, Size, io::Result<Hash>)>(threads);

  // n workers
  for _ in 0..threads {
    let inputs_rx = inputs_rx.clone();
    let outputs_tx = outputs_tx.clone();
    thread::spawn(move || {
      for (path, size) in inputs_rx {
        let hash = fs::hash(&path);
        let _ = outputs_tx.send((path, size, hash));
      }
    });
  }

  drop(inputs_rx);
  drop(outputs_tx);

  // producer
  thread::spawn(move || {
    for (path, size) in sizes {
      let _ = inputs_tx.send((path, size));
    }
  });

  // consumer
  writeln!(w, "computing hashes… \x1b[2m({threads} threads)\x1b[22m")?;
  write!(w, "files processed: \x1b[93m\x1b[s\x1b[?25l")?; // save and hide cursor
  for (path, size, hash) in outputs_rx {
    match hash {
      Err(err) => errors.push((path, err)),
      Ok(hash) => {
        let full_hash = (size, hash);
        hashes.push((path, full_hash));
        *hash_counts.entry(full_hash).or_default() += 1;
        write!(w, "\x1b[u{}\x1b[K", hashes.len())?;
      }
    };
  }
  writeln!(w, "\x1b[?25h\x1b[39m")
}

fn show_duplicate(mut w: impl Write, hashes: Hashes, root: &Path) -> io::Result<()> {
  let mut path_groups = HashMap::<_, Vec<_>>::default();
  for (path, hash) in hashes {
    path_groups.entry(hash).or_default().push(path);
  }

  let mut path_groups = Vec::from_iter(path_groups);
  path_groups.sort_unstable_by(|((size_a, _), a), ((size_b, _), b)| {
    let bytes_dup_a = a.len() as Size * size_a - size_a;
    let bytes_dup_b = b.len() as Size * size_b - size_b;
    bytes_dup_a.cmp(&bytes_dup_b) // sort by wasted space
  });

  for ((size, hash), mut paths) in path_groups {
    let len @ 2.. = paths.len() else { continue };
    let mid = len.min(3);

    let each = fmt::Size(size);
    let dup = fmt::Size(len as Size * size - size);

    let hash = format_args!("\x1b[96m{hash:016x}\x1b[39m");
    let count = format_args!("\x1b[93m{len}\x1b[39m files");
    let each = format_args!("\x1b[93m{each}\x1b[39m each");
    let dup = format_args!("\x1b[93m{dup}\x1b[39m duplicated");

    writeln!(w)?;
    writeln!(w, "{hash}{0}{count}{0}{each}{0}{dup}", " \x1b[2m·\x1b[22m ")?;

    let cmp = |a: &PathBuf, b: &PathBuf| {
      let [ac, bc] = [a, b].map(|p| p.components().count()); // by component count
      let [an, bn] = [a, b].map(|p| p.as_os_str().len()); // then by length
      ac.cmp(&bc).then(an.cmp(&bn)).then(a.cmp(b)) // then lexicographically
    };

    let (show, hide) = {
      let (slice, _, _) = paths.select_nth_unstable_by(mid - 1, cmp);
      slice.sort_unstable_by(cmp);
      paths.split_at(mid)
    };

    for (i, path) in show.iter().enumerate() {
      let path = path.strip_prefix(root).unwrap_or(path).display();
      match i {
        0 => writeln!(w, "{path}")?,
        _ => writeln!(w, "\x1b[2m{path}\x1b[22m")?,
      }
    }

    if let n @ 1.. = hide.len() {
      writeln!(w, "\x1b[2mand {n} more…\x1b[22m")?;
    }
  }

  Ok(())
}

fn show_errors(mut w: impl Write, mut errors: Errors, root: &Path) -> io::Result<()> {
  let 1.. = errors.len() else { return Ok(()) };

  let key = |(_, e): &(_, io::Error)| (e.kind(), e.raw_os_error());
  let grouped = {
    errors.sort_unstable_by_key(key);
    errors.chunk_by(|a, b| key(a) == key(b))
  };

  writeln!(w)?;
  writeln!(w, "\x1b[91merrors:\x1b[39m")?;
  for group in grouped {
    let (_, err) = &group[0];
    writeln!(w, "{err}:")?;

    let (show, hide) = group.split_at(group.len().min(3));
    for (path, _) in show {
      let path = path.strip_prefix(root).unwrap_or(path).display();
      writeln!(w, "\x1b[2m{path}\x1b[22m")?;
    }
    if let n @ 1.. = hide.len() {
      writeln!(w, "\x1b[2mand {n} more…\x1b[22m")?;
    }
  }

  Ok(())
}

fn show_summary(mut w: impl Write, hash_counts: HashCounts, s: f64) -> io::Result<()> {
  let (mut total, mut total_n) = (0, 0);
  let (mut hashed, mut hashed_n) = (0, 0);
  let (mut dup, mut dup_n) = (0, 0);

  for ((size, hash), n) in hash_counts {
    total_n += n;
    total += n * size;

    if hash != 0 {
      hashed_n += n;
      hashed += n * size;
    }

    if n > 1 {
      dup_n += n - 1;
      dup += n * size - size;
    }
  }

  let (uniq, uniq_n) = (total - dup, total_n - dup_n);
  let uniq_pct_n = fmt::percentage(uniq_n, total_n);
  let uniq_pct = fmt::percentage(uniq, total);
  let dup_pct_n = fmt::percentage(dup_n, total_n);
  let dup_pct = fmt::percentage(dup, total);
  let rate_n = hashed_n as f64 / s;
  let rate = fmt::Size((hashed as f64 / s) as Size);
  let total = fmt::Size(total);
  let hashed = fmt::Size(hashed);
  let dup = fmt::Size(dup);
  let uniq = fmt::Size(uniq);

  let total_n = format_args!("\x1b[93m{total_n}\x1b[39m");
  let uniq_n = format_args!("\x1b[92m{uniq_n}\x1b[39m \x1b[2m({uniq_pct_n:.0}%)\x1b[22m");
  let dup_n = format_args!("\x1b[91m{dup_n}\x1b[39m \x1b[2m({dup_pct_n:.0}%)\x1b[22m");

  let total = format_args!("\x1b[93m{total}\x1b[39m");
  let uniq = format_args!("\x1b[92m{uniq}\x1b[39m \x1b[2m({uniq_pct:.0}%)\x1b[22m");
  let dup = format_args!("\x1b[91m{dup}\x1b[39m \x1b[2m({dup_pct:.0}%)\x1b[22m");

  let time = format_args!("computed \x1b[93m{hashed_n}\x1b[39m hashes in \x1b[93m{s:.2}s\x1b[39m");
  let stats = format_args!("\x1b[2m({rate_n:.0} files/s · {rate}/s · {hashed})\x1b[22m");

  writeln!(w)?;
  writeln!(w, "{total_n} files: {uniq_n} unique and {dup_n} duplicated")?;
  writeln!(w, "{total} of data: {uniq} unique and {dup} duplicated")?;
  writeln!(w)?;
  writeln!(w, "{time} {stats}")
}
