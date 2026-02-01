use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};
use std::{env, io, io::prelude::*, mem, thread};

use crossbeam_channel as channel;

use self::stdx::fs::FileHash;
use self::stdx::{ansi, fmt, fs};

mod stdx;

type Sig = [u64; 2]; // [meta bits | file size, file hash]
type Sigs = Vec<(Box<Path>, Sig)>;
type SigCount = HashMap<Sig, u64>;

type IsFullPath = bool;
type Errors = Vec<(Box<Path>, IsFullPath, io::Error)>;

const PARTIAL_HASH_SIZE: u64 = 1 << 12; // 4 KiB

const HASH_PREF: u64 = 0b100 << 61;
const HASH_SUFF: u64 = 0b010 << 61;
const HASH_FULL: u64 = 0b001 << 61;
const HASH_MASK: u64 = 0b111 << 61;
const SIZE_MASK: u64 = !HASH_MASK;

fn main() -> io::Result<()> {
  ansi::enable();

  let path = env::args_os().nth(1).unwrap_or_else(|| ".".into());
  let path = Path::new(&path);
  let mut w = io::stdout().lock();
  let (mut errs, mut inputs, mut sigs, mut sig_count) = Default::default();

  scan(&mut w, &mut errs, &mut inputs, path)?;
  filter_and_count(&mut w, &mut inputs, &mut sigs, &mut sig_count)?;

  let prefix = compute_hashes(&mut w, &mut errs, &mut inputs, FileHash::Prefix(PARTIAL_HASH_SIZE))?;
  filter_and_count(&mut w, &mut inputs, &mut sigs, &mut sig_count)?;

  let suffix = compute_hashes(&mut w, &mut errs, &mut inputs, FileHash::Suffix(PARTIAL_HASH_SIZE))?;
  filter_and_count(&mut w, &mut inputs, &mut sigs, &mut sig_count)?;

  let full = compute_hashes(&mut w, &mut errs, &mut inputs, FileHash::Full)?;
  count(inputs, &mut sigs, &mut sig_count);

  show_duplicates(&mut w, sigs, path)?;
  show_errors(&mut w, errs, path)?;
  show_summary(&mut w, sig_count, [prefix, suffix, full])
}

fn scan(mut w: impl Write, errs: &mut Errors, sigs: &mut Sigs, root: &Path) -> io::Result<()> {
  writeln!(w, "scanning file system…")?;
  write!(w, "files found: \x1b[s\x1b[?25l")?; // save and hide cursor
  let mut total = 0;
  fs::scan(root, &mut |cwd, r| {
    let r = r.map_err(|err| (cwd.to_owned().into_boxed_path(), false, err));
    let r = r.and_then(|(path, file)| match file.metadata() {
      Err(err) => Err((path.into_boxed_path(), true, err)),
      Ok(meta) => Ok((path, meta)),
    });
    match r {
      Err(err) => errs.push(err),
      Ok((path, meta)) => {
        total += meta.len();
        sigs.push((path.into_boxed_path(), [meta.len(), 0]));
        write!(w, "\x1b[u\x1b[93m{}\x1b[39m \x1b[2m({})\x1b[22m\x1b[K", sigs.len(), fmt::Size(total))?;
      }
    };
    Ok(())
  })?;
  writeln!(w, "\x1b[?25h")
}

fn filter_and_count(mut w: impl Write, inputs: &mut Sigs, sigs: &mut Sigs, sig_count: &mut SigCount) -> io::Result<()> {
  let inputs_sig_count = inputs.iter().fold(SigCount::default(), |mut acc, &(_, sig)| {
    *acc.entry(sig).or_default() += 1;
    acc
  });

  let total = inputs.len();
  let skip = inputs.extract_if(.., |&mut (_, sig @ [meta, _])| {
    let [hash, size] = [meta & HASH_MASK, meta & SIZE_MASK];
    size == 0 || // empty file
      size <= PARTIAL_HASH_SIZE && hash != 0 || // already fully hashed small file
      inputs_sig_count.get(&sig).is_some_and(|&n| n < 2) // unique file
  });
  count(skip, sigs, sig_count);

  if let n @ 1.. = total - inputs.len() {
    writeln!(w)?;
    writeln!(w, "files skipped: \x1b[92m{n}\x1b[39m \x1b[2m(unique, empty, or already hashed)\x1b[22m")?;
  }

  Ok(())
}

fn count(inputs: impl IntoIterator<Item = (Box<Path>, Sig)>, sigs: &mut Sigs, sig_count: &mut SigCount) {
  for (path, sig) in inputs {
    sigs.push((path, sig));
    *sig_count.entry(sig).or_default() += 1;
  }
}

fn compute_hashes(mut w: impl Write, errs: &mut Errors, sigs: &mut Sigs, hash: FileHash) -> io::Result<Duration> {
  let 1.. = sigs.len() else { return Ok(Default::default()) };

  let (hash_type, hash_name) = match hash {
    FileHash::Full => (HASH_FULL, "full"),
    FileHash::Prefix(_) => (HASH_PREF, "prefix"),
    FileHash::Suffix(_) => (HASH_SUFF, "suffix"),
  };

  let threads_n = thread::available_parallelism()?.get();
  let (inputs_tx, inputs_rx) = channel::bounded::<(Box<Path>, Sig)>(threads_n << 8);
  let (outputs_tx, outputs_rx) = channel::bounded::<(Box<Path>, io::Result<Sig>)>(threads_n << 8);

  // n workers
  for _ in 0..threads_n {
    let inputs_rx = inputs_rx.clone();
    let outputs_tx = outputs_tx.clone();
    thread::spawn(move || {
      for (path, [meta, _]) in inputs_rx {
        let sig = hash.compute(&path).map(|hash| [meta | hash_type, hash]);
        let _ = outputs_tx.send((path, sig));
      }
    });
  }

  drop(inputs_rx);
  drop(outputs_tx);

  // producer
  let inputs = mem::take(sigs);
  let count = inputs.len();
  thread::spawn(move || {
    for input in inputs {
      let _ = inputs_tx.send(input);
    }
  });

  // consumer
  writeln!(w)?;
  writeln!(w, "computing \x1b[93m{count}\x1b[39m {hash_name} hashes… \x1b[2m({threads_n} threads)\x1b[22m")?;
  write!(w, "computed: \x1b[93m\x1b[s\x1b[?25l")?; // save and hide cursor
  let t0 = Instant::now();
  for (path, sig) in outputs_rx {
    match sig {
      Err(err) => errs.push((path, true, err)),
      Ok(sig) => {
        sigs.push((path, sig));
        write!(w, "\x1b[u{}\x1b[K", sigs.len())?;
      }
    };
  }
  let t1 = Instant::now();
  writeln!(w, "\x1b[?25h\x1b[39m")?;

  Ok(t1 - t0)
}

fn show_duplicates(mut w: impl Write, sigs: Sigs, root: &Path) -> io::Result<()> {
  let mut path_groups = HashMap::<_, Vec<_>>::default();
  for (path, sig) in sigs {
    path_groups.entry(sig).or_default().push(path);
  }

  let mut path_groups = Vec::from_iter(path_groups);
  path_groups.sort_unstable_by(|([meta0, _], group0), ([meta1, _], group1)| {
    let a = (group0.len() as u64 - 1) * (meta0 & SIZE_MASK);
    let b = (group1.len() as u64 - 1) * (meta1 & SIZE_MASK);
    a.cmp(&b) // by total duplicated bytes
  });

  for ([meta, hash], mut group) in path_groups {
    let len @ 2.. = group.len() else { continue };
    let mid = len.min(3);

    let size = meta & SIZE_MASK;
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
      let (slice, _, _) = group.select_nth_unstable_by(mid - 1, cmp);
      slice.sort_unstable_by(cmp);
      group.split_at(mid)
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

fn show_errors(mut w: impl Write, mut errs: Errors, root: &Path) -> io::Result<()> {
  let 1.. = errs.len() else { return Ok(()) };

  errs.sort_unstable_by(|(p0, _, e0), (p1, _, e1)| {
    let a = (e0.kind(), e0.raw_os_error(), p0);
    let b = (e1.kind(), e1.raw_os_error(), p1);
    a.cmp(&b) // by error kind, by OS error code, lexicographically by path
  });

  let groups = errs.chunk_by(|(_, _, e0), (_, _, e1)| {
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

fn show_summary(mut w: impl Write, sig_count: SigCount, [prefix_t, suffix_t, full_t]: [Duration; 3]) -> io::Result<()> {
  let (mut total_n, mut skipped_n, mut prefix_n, mut suffix_n, mut full_n, mut dup_n) = (0, 0, 0, 0, 0, 0);
  let (mut total, mut skipped, mut prefix, mut suffix, mut full, mut dup) = (0, 0, 0, 0, 0, 0);

  for ([meta, _], count) in sig_count {
    let size = meta & SIZE_MASK;
    total_n += count;
    total += count * size;
    if count > 1 {
      dup_n += count - 1;
      dup += (count - 1) * size;
    }
    if meta & HASH_MASK == 0 {
      skipped_n += count;
      skipped += count * size;
    }
    if meta & HASH_PREF != 0 {
      prefix_n += count;
      prefix += count * PARTIAL_HASH_SIZE;
    }
    if meta & HASH_SUFF != 0 {
      suffix_n += count;
      suffix += count * PARTIAL_HASH_SIZE;
    }
    if meta & HASH_FULL != 0 {
      full_n += count;
      full += count * size;
    }
  }

  let (uniq, uniq_n) = (total - dup, total_n - dup_n);
  let (uniq_pct, uniq_n_pct) = (fmt::percentage(uniq, total), fmt::percentage(uniq_n, total_n));
  let (dup_pct, dup_n_pct) = (fmt::percentage(dup, total), fmt::percentage(dup_n, total_n));
  let (total, uniq, dup, skipped) = (fmt::Size(total), fmt::Size(uniq), fmt::Size(dup), fmt::Size(skipped));

  {
    let total_n = format_args!("\x1b[96m{total_n}\x1b[39m");
    let uniq_n = format_args!("\x1b[92m{uniq_n}\x1b[39m \x1b[2m({uniq_n_pct:.0}%)\x1b[22m");
    let dup_n = format_args!("\x1b[93m{dup_n}\x1b[39m \x1b[2m({dup_n_pct:.0}%)\x1b[22m");
    writeln!(w)?;
    writeln!(w, "{total_n} files: {uniq_n} unique and {dup_n} duplicates")?;

    let total = format_args!("\x1b[96m{total}\x1b[39m");
    let uniq = format_args!("\x1b[92m{uniq}\x1b[39m \x1b[2m({uniq_pct:.0}%)\x1b[22m");
    let dup = format_args!("\x1b[93m{dup}\x1b[39m \x1b[2m({dup_pct:.0}%)\x1b[22m");
    writeln!(w)?;
    writeln!(w, "{total} of data: {uniq} unique and {dup} duplicated")?;
  }

  if total_n > skipped_n {
    writeln!(w)?;
    writeln!(w, "skipped \x1b[96m{skipped_n}\x1b[39m files \x1b[2m({skipped})\x1b[22m")?;

    let prefix = (2, "prefix", prefix, prefix_n, prefix_t);
    let suffix = (2, "suffix", suffix, suffix_n, suffix_t);
    let full = (3, "full", full, full_n, full_t);
    for (color, name, bytes, n, t) in [prefix, suffix, full] {
      let s = t.as_secs_f64().max(f64::MIN_POSITIVE);

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
