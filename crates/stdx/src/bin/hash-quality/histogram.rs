use std::fmt;

pub struct Histogram<const N: usize> {
  bins: [u32; N],
  step: f64,
  min: f64,
}

impl<const N: usize> Histogram<N> {
  pub fn new(min: f64, max: f64) -> Self {
    let step = (max - min) / (N - 1) as f64;
    Self { bins: [0; N], min, step }
  }

  pub fn centered(mean: f64, margin: f64) -> Self {
    Self::new(mean - margin, mean + margin)
  }
}

impl<const N: usize> Extend<f64> for Histogram<N> {
  fn extend<T: IntoIterator<Item = f64>>(&mut self, xs: T) {
    let inv_step = 1. / self.step;
    for x in xs {
      let i = ((x - self.min) * inv_step).round();
      if let Some(count) = self.bins.get_mut(i as isize as usize) {
        *count += 1;
      }
    }
  }
}

impl<const N: usize> fmt::Display for Histogram<N> {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let max = self.min + self.step * (N - 1) as f64;
    let max_count = self.bins.iter().fold(1, |acc, &n| acc.max(n));

    let xw = 3 + max.log10() as usize;
    let cw = 2 + max_count.ilog10() as usize;

    for (i, &count) in self.bins.iter().enumerate() {
      let x = self.min + (self.step * i as f64);

      let lin = (100 * count / max_count) as usize;
      let log = (u32::BITS - count.leading_zeros()) as usize;

      let both = lin.min(log);
      let fg = lin.saturating_sub(both);
      let bg = log.saturating_sub(both);

      let both = format_args!("\x1b[2m█\x1b[40m{:▀>both$}", "");
      let either = match [fg, bg] {
        [0, 0] => format_args!("\x1b[49;22m"),
        [_, 0] => format_args!("\x1b[49m{:▀>fg$}\x1b[22m", ""),
        [_, _] => format_args!("\x1b[22m{: >bg$}\x1b[49m", ""),
      };

      writeln!(f, "\x1b[94m{x:xw$.1}\x1b[93m{count:cw$}\x1b[39m {both}{either}")?;
    }

    Ok(())
  }
}
