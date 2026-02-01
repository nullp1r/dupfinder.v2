use std::collections::hash_map::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};
use std::usize;
use std::{env, io, io::prelude::*, mem, thread};

use crossbeam_channel as channel;

use self::stdx::{ansi, fmt, fs};

mod stdx;

type Sig = [u64; 2]; // [meta bits | file size, file hash]
type Sigs = Vec<(Box<Path>, Sig)>;
type SigCount = HashMap<Sig, u64>;

type IsFullPath = bool;
type Errors = Vec<(Box<Path>, IsFullPath, io::Error)>;

mod bits {
  pub const HASH_FAST: u64 = 0b10 << 62;
  pub const HASH_SLOW: u64 = 0b01 << 62;
  pub const HASH: u64 = 0b11 << 62;
  pub const SIZE: u64 = !HASH;
}

fn main() -> io::Result<()> {
  ansi::enable();

  let path = env::args_os().nth(1).unwrap_or_else(|| ".".into());
  let path = Path::new(&path);
  let mut w = io::stdout().lock();
  let (mut errs, mut inputs, mut sigs, mut sig_count) = Default::default();

  scan(&mut w, &mut errs, &mut inputs, path)?;
  filter_and_count(&mut w, &mut inputs, &mut sigs, &mut sig_count)?;

  let fast = compute_hashes(&mut w, &mut errs, &mut inputs, 1)?;
  filter_and_count(&mut w, &mut inputs, &mut sigs, &mut sig_count)?;

  let slow = compute_hashes(&mut w, &mut errs, &mut inputs, usize::MAX)?;
  count(inputs, &mut sigs, &mut sig_count);

  show_duplicates(&mut w, sigs, path)?;
  show_errors(&mut w, errs, path)?;
  show_summary(&mut w, sig_count, [fast, slow])
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
    let empty = meta & bits::SIZE == 0;
    let unique = inputs_sig_count.get(&sig).is_some_and(|&n| n < 2);
    empty || unique
  });
  count(skip, sigs, sig_count);

  if let n @ 1.. = total - inputs.len() {
    writeln!(w)?;
    writeln!(w, "files skipped: \x1b[92m{n}\x1b[39m \x1b[2m(unique or empty)\x1b[22m")?;
  }

  Ok(())
}

fn count(inputs: impl IntoIterator<Item = (Box<Path>, Sig)>, sigs: &mut Sigs, sig_count: &mut SigCount) {
  for (path, sig) in inputs {
    sigs.push((path, sig));
    *sig_count.entry(sig).or_default() += 1;
  }
}

fn compute_hashes(mut w: impl Write, errs: &mut Errors, sigs: &mut Sigs, steps: usize) -> io::Result<Duration> {
  let 1.. = sigs.len() else { return Ok(Default::default()) };

  let threads = thread::available_parallelism()?.get();
  let (inputs_tx, inputs_rx) = channel::bounded::<(Box<Path>, Sig)>(threads << 8);
  let (outputs_tx, outputs_rx) = channel::bounded::<(Box<Path>, io::Result<Sig>)>(threads << 8);

  let (hash_type, hash) = match steps {
    1 => (bits::HASH_FAST, "fast"),
    _ => (bits::HASH_SLOW, "slow"),
  };

  // n workers
  for _ in 0..threads {
    let inputs_rx = inputs_rx.clone();
    let outputs_tx = outputs_tx.clone();
    thread::spawn(move || {
      for (path, [meta, _]) in inputs_rx {
        let sig = match fs::hash(&path, steps) {
          Ok(hash) => Ok([meta | hash_type, hash]),
          Err(err) => Err(err),
        };
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
  writeln!(w, "computing \x1b[93m{count}\x1b[39m {hash} hashes… \x1b[2m({threads} threads)\x1b[22m")?;
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
    let a = (group0.len() as u64 - 1) * (meta0 & bits::SIZE);
    let b = (group1.len() as u64 - 1) * (meta1 & bits::SIZE);
    a.cmp(&b) // by total duplicated bytes
  });

  for ([meta, hash], mut group) in path_groups {
    let len @ 2.. = group.len() else { continue };
    let mid = len.min(3);

    let size = meta & bits::SIZE;
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

fn show_summary(mut w: impl Write, sig_count: SigCount, [fast_t, slow_t]: [Duration; 2]) -> io::Result<()> {
  let (mut total_n, mut skipped_n, mut fast_n, mut slow_n, mut dup_n) = (0, 0, 0, 0, 0);
  let (mut total, mut skipped, mut fast, mut slow, mut dup) = (0, 0, 0, 0, 0);

  for ([meta, _], count) in sig_count {
    let size = meta & bits::SIZE;
    total_n += count;
    total += count * size;
    if count > 1 {
      dup_n += count - 1;
      dup += (count - 1) * size;
    }
    if meta & bits::HASH == 0 {
      skipped_n += count;
      skipped += count * size;
    }
    if meta & bits::HASH_FAST != 0 {
      fast_n += count;
      fast += count * fs::BUF_SIZE_FAST as u64;
    }
    if meta & bits::HASH_SLOW != 0 {
      slow_n += count;
      slow += count * size;
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

    let fast = (2, "fast", fast, fast_n, fast_t);
    let slow = (3, "slow", slow, slow_n, slow_t);
    for (color, name, bytes, n, t) in [fast, slow] {
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
