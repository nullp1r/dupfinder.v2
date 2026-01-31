use std::collections::hash_map::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};
use std::usize;
use std::{env, io, io::prelude::*, mem, thread};

use crossbeam_channel as channel;

use self::stdx::{ansi, fmt, fs};

mod stdx;

type Hash = [u64; 2]; // [file hash, file size and bitflags]

type Counts = HashMap<Hash, u64>;
type Hashes = Vec<(Box<Path>, Hash)>;
type Errors = Vec<(Box<Path>, bool, io::Error)>; // `true` if full path

mod bits {
  pub const HASH_FAST: u64 = 0b10 << 62;
  pub const HASH_SLOW: u64 = 0b01 << 62;
  pub const HASH: u64 = 0b11 << 62;
  pub const SIZE: u64 = !HASH;
}

fn main() -> io::Result<()> {
  let _ = ansi::enable();

  let path = env::args_os().nth(1).unwrap_or_else(|| ".".into());
  let path = Path::new(&path);
  let mut w = io::stdout().lock();
  let (mut errors, mut inputs, mut hashes, mut counts) = Default::default();

  scan(&mut w, &mut errors, &mut inputs, path)?;
  filter_and_count(&mut w, &mut inputs, &mut hashes, &mut counts)?;

  let fast = compute_hashes(&mut w, &mut errors, &mut inputs, 1)?;
  filter_and_count(&mut w, &mut inputs, &mut hashes, &mut counts)?;

  let slow = compute_hashes(&mut w, &mut errors, &mut inputs, usize::MAX)?;
  count(inputs, &mut hashes, &mut counts);

  show_duplicates(&mut w, hashes, path)?;
  show_errors(&mut w, errors, path)?;
  show_summary(&mut w, counts, [fast, slow])
}

fn scan(mut w: impl Write, errors: &mut Errors, sizes: &mut Hashes, root: &Path) -> io::Result<()> {
  writeln!(w, "scanning file system…")?;
  write!(w, "files found: \x1b[s\x1b[?25l")?; // save and hide cursor
  let mut total = 0;

  fs::scan(root, &mut |cwd, r| {
    let r = r.map_err(|err| (cwd.to_owned().into_boxed_path(), false, err));
    let r = r.and_then(|(path, file)| match file.metadata() {
      Err(err) => Err((path.into_boxed_path(), true, err)),
      Ok(meta) => Ok((path, meta.len())),
    });
    match r {
      Err(err) => errors.push(err),
      Ok((path, size)) => {
        total += size;
        sizes.push((path.into_boxed_path(), [0, size]));
        write!(w, "\x1b[u\x1b[93m{}\x1b[39m \x1b[2m({})\x1b[22m\x1b[K", sizes.len(), fmt::Size(total))?;
      }
    };
    Ok(())
  })?;
  writeln!(w, "\x1b[?25h")
}

fn filter_and_count(mut w: impl Write, inputs: &mut Hashes, hashes: &mut Hashes, hash_counts: &mut Counts) -> io::Result<()> {
  let counts = inputs.iter().fold(Counts::default(), |mut acc, &(_, hash)| {
    *acc.entry(hash).or_default() += 1;
    acc
  });

  let total = inputs.len();
  let skip = inputs.extract_if(.., |&mut (_, hash @ [_, bits])| {
    let empty = bits & bits::SIZE == 0;
    let unique = counts.get(&hash).is_some_and(|&n| n < 2);
    empty || unique
  });
  count(skip, hashes, hash_counts);

  if let n @ 1.. = total - inputs.len() {
    writeln!(w)?;
    writeln!(w, "skipped \x1b[92m{n}\x1b[39m files \x1b[2m(unique or empty)\x1b[22m")?;
  }

  Ok(())
}

fn count(inputs: impl IntoIterator<Item = (Box<Path>, Hash)>, hashes: &mut Hashes, hash_counts: &mut Counts) {
  for (path, hash) in inputs {
    hashes.push((path, hash));
    *hash_counts.entry(hash).or_default() += 1;
  }
}

fn compute_hashes(mut w: impl Write, errors: &mut Errors, hashes: &mut Hashes, steps: usize) -> io::Result<Duration> {
  let 1.. = hashes.len() else { return Ok(Default::default()) };

  let threads = thread::available_parallelism()?.get();
  let (inputs_tx, inputs_rx) = channel::bounded::<(Box<Path>, Hash)>(threads << 8);
  let (outputs_tx, outputs_rx) = channel::bounded::<(Box<Path>, io::Result<Hash>)>(threads << 8);

  let (hash_bit, hash_type) = match steps {
    1 => (bits::HASH_FAST, "fast"),
    _ => (bits::HASH_SLOW, "slow"),
  };

  // n workers
  for _ in 0..threads {
    let inputs_rx = inputs_rx.clone();
    let outputs_tx = outputs_tx.clone();
    thread::spawn(move || {
      for (path, [_, bits]) in inputs_rx {
        let hash = match fs::hash(&path, steps) {
          Ok(hash) => Ok([hash, hash_bit | bits]),
          Err(err) => Err(err),
        };
        let _ = outputs_tx.send((path, hash));
      }
    });
  }

  drop(inputs_rx);
  drop(outputs_tx);

  // producer
  let inputs = mem::take(hashes);
  let count = inputs.len();
  thread::spawn(move || {
    for (path, hash) in inputs {
      let _ = inputs_tx.send((path, hash));
    }
  });

  // consumer
  writeln!(w)?;
  writeln!(w, "computing \x1b[93m{count}\x1b[39m {hash_type} hashes… \x1b[2m({threads} threads)\x1b[22m")?;
  write!(w, "computed: \x1b[93m\x1b[s\x1b[?25l")?; // save and hide cursor
  let t0 = Instant::now();
  for (path, hash) in outputs_rx {
    match hash {
      Err(err) => errors.push((path, true, err)),
      Ok(hash) => {
        hashes.push((path, hash));
        write!(w, "\x1b[u{}\x1b[K", hashes.len())?;
      }
    };
  }
  let t1 = Instant::now();
  writeln!(w, "\x1b[?25h\x1b[39m")?;

  Ok(t1 - t0)
}

fn show_duplicates(mut w: impl Write, hashes: Hashes, root: &Path) -> io::Result<()> {
  let mut path_groups = HashMap::<_, Vec<_>>::default();
  for (path, hash) in hashes {
    path_groups.entry(hash).or_default().push(path);
  }

  let mut path_groups = Vec::from_iter(path_groups);
  path_groups.sort_unstable_by(|([_, bits0], pg0), ([_, bits1], pg1)| {
    let a = (pg0.len() as u64 - 1) * (bits0 & bits::SIZE);
    let b = (pg1.len() as u64 - 1) * (bits1 & bits::SIZE);
    a.cmp(&b) // by total duplicated bytes
  });

  for ([hash, bits], mut paths) in path_groups {
    let len @ 2.. = paths.len() else { continue };
    let mid = len.min(3);

    let size = bits & bits::SIZE;
    let each = fmt::Size(size);
    let dup = fmt::Size(size * (len as u64 - 1));

    let hash = format_args!("\x1b[96m{hash:016x}\x1b[39m");
    let count = format_args!("\x1b[93m{len}\x1b[39m files");
    let each = format_args!("\x1b[93m{each}\x1b[39m each");
    let dup = format_args!("\x1b[93m{dup}\x1b[39m duplicated");

    writeln!(w)?;
    writeln!(w, "{hash}{0}{count}{0}{each}{0}{dup}", " \x1b[2m·\x1b[22m ")?;

    let cmp = |p0: &Box<Path>, p1: &Box<Path>| {
      let [c0, c1] = [p0, p1].map(|p| p.components().count());
      let [n0, n1] = [p0, p1].map(|p| p.as_os_str().len());
      (c0, n0, p0).cmp(&(c1, n1, p1)) // by depth, by length, lexicographically
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

  errors.sort_unstable_by(|(p0, _, e0), (p1, _, e1)| {
    let a = (e0.kind(), e0.raw_os_error(), p0);
    let b = (e1.kind(), e1.raw_os_error(), p1);
    a.cmp(&b) // by error kind, by OS error code, lexicographically by path
  });

  let groups = errors.chunk_by(|(_, _, e0), (_, _, e1)| {
    let a = (e0.kind(), e0.raw_os_error());
    let b = (e1.kind(), e1.raw_os_error());
    a == b // group by error kind and OS error code
  });

  for group in groups {
    let (_, _, err) = &group[0];
    writeln!(w)?;
    writeln!(w, "\x1b[91m{err}:\x1b[39m")?;

    let (show, hide) = group.split_at(group.len().min(3));

    for (path, full_path, _) in show {
      let [c0, c1] = if *full_path { ["", ""] } else { ["\x1b[2m", "\x1b[22m"] };
      let path = path.strip_prefix(root).unwrap_or(path);
      let path = if let 0 = path.as_os_str().len() { root } else { path };
      writeln!(w, "{c0}{}{c1}", path.display())?
    }

    if let n @ 1.. = hide.len() {
      writeln!(w, "\x1b[2mand {n} more…\x1b[22m")?;
    }
  }

  Ok(())
}

fn show_summary(mut w: impl Write, counts: Counts, [fast_t, slow_t]: [Duration; 2]) -> io::Result<()> {
  let (mut total_n, mut skipped_n, mut fast_n, mut slow_n, mut dup_n) = (0, 0, 0, 0, 0);
  let (mut total, mut skipped, mut fast, mut slow, mut dup) = (0, 0, 0, 0, 0);

  for ([_, bits], count) in counts {
    let size = bits & bits::SIZE;
    total_n += count;
    total += count * size;
    if count > 1 {
      dup_n += count - 1;
      dup += (count - 1) * size;
    }
    if bits & bits::HASH == 0 {
      skipped_n += count;
      skipped += count * size;
    }
    if bits & bits::HASH_FAST != 0 {
      fast_n += count;
      fast += count * fs::BUF_SIZE_FAST as u64;
    }
    if bits & bits::HASH_SLOW != 0 {
      slow_n += count;
      slow += count * size;
    }
  }

  let (uniq, uniq_n) = (total - dup, total_n - dup_n);
  let (uniq_pct, uniq_n_pct) = (fmt::percentage(uniq, total), fmt::percentage(uniq_n, total_n));
  let (dup_pct, dup_n_pct) = (fmt::percentage(dup, total), fmt::percentage(dup_n, total_n));
  let (total, uniq, dup, skipped) = (fmt::Size(total), fmt::Size(uniq), fmt::Size(dup), fmt::Size(skipped));

  {
    let total_n = format_args!("\x1b[93m{total_n}\x1b[39m");
    let uniq_n = format_args!("\x1b[92m{uniq_n}\x1b[39m \x1b[2m({uniq_n_pct:.0}%)\x1b[22m");
    let dup_n = format_args!("\x1b[91m{dup_n}\x1b[39m \x1b[2m({dup_n_pct:.0}%)\x1b[22m");
    writeln!(w)?;
    writeln!(w, "{total_n} files: {uniq_n} unique and {dup_n} duplicates")?;

    let total = format_args!("\x1b[93m{total}\x1b[39m");
    let uniq = format_args!("\x1b[92m{uniq}\x1b[39m \x1b[2m({uniq_pct:.0}%)\x1b[22m");
    let dup = format_args!("\x1b[91m{dup}\x1b[39m \x1b[2m({dup_pct:.0}%)\x1b[22m");
    writeln!(w)?;
    writeln!(w, "{total} of data: {uniq} unique and {dup} duplicated")?;
  }

  if total_n > skipped_n {
    writeln!(w)?;
    writeln!(w, "skipped \x1b[92m{skipped_n}\x1b[39m files \x1b[2m({skipped})\x1b[22m")?;

    let fast = (4, "fast", fast, fast_n, fast_t);
    let slow = (5, "slow", slow, slow_n, slow_t);
    for (color, name, bytes, n, t) in [fast, slow] {
      let s = t.as_secs_f64();

      let size = fmt::Size(bytes);
      let rate = fmt::Size((bytes as f64 / s) as u64);
      let rate_n = n as f64 / s;

      let perf = format_args!("computed \x1b[9{color}m{n}\x1b[39m {name} hashes in \x1b[93m{s:.2}s\x1b[39m");
      let stats = format_args!("\x1b[2m({rate_n:.0} files/s · {rate}/s · {size})\x1b[22m");
      writeln!(w, "{perf} {stats}")?;
    }
  }

  Ok(())
}
