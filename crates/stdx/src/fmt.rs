use std::fmt;

pub struct Size(pub u64); // bytes
pub struct Time(pub u64); // nanoseconds

impl fmt::Display for Size {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let [s, u] = f.width().map_or([0, 0], |w| [w, 2]);
    let i = self.0.max(1).ilog2() as usize / 10;
    let unit = ["", "Ki", "Mi", "Gi", "Ti", "Pi", "Ei"][i];
    let size = self.0 as f64 / (1u64 << 10 * i) as f64;
    let prec = i.min(1) * precision::<3>(size);
    write!(f, "{size:s$.prec$} {unit:u$}B")
  }
}

impl fmt::Display for Time {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let [t, u] = f.width().map_or([0, 0], |w| [w, 1]);
    let i = self.0.max(1).ilog10().min(9) as usize / 3;
    let unit = ["n", "µ", "m", ""][i];
    let time = self.0 as f64 / [1e0, 1e3, 1e6, 1e9][i];
    let prec = i.min(1) * precision::<3>(time);
    write!(f, "{time:t$.prec$} {unit:u$}s")
  }
}

fn precision<const N: u32>(n: f64) -> usize {
  let num = 10u64.pow(N + 1) - 5;
  (2..=N).fold(0, |acc, i| {
    let threshold = num as f64 / 10u64.pow(i) as f64;
    acc + (threshold > n) as usize
  })
}
