#[derive(Default)]
pub struct Welford {
  count: u64,
  mean: f64,
  m2: f64,
}

impl Welford {
  pub fn stats(&self, sample: bool) -> [f64; 3] {
    let div = self.count.saturating_sub(sample as u64).max(1);
    let var = self.m2 / div as f64;
    [self.mean, var, var.sqrt()]
  }
}

impl Extend<f64> for Welford {
  fn extend<T: IntoIterator<Item = f64>>(&mut self, xs: T) {
    for x in xs {
      let delta = x - self.mean;
      self.count += 1;
      self.mean = delta.mul_add(1. / self.count as f64, self.mean);
      self.m2 = delta.mul_add(x - self.mean, self.m2);
    }
  }
}

impl FromIterator<f64> for Welford {
  fn from_iter<T: IntoIterator<Item = f64>>(xs: T) -> Self {
    let mut w = Self::default();
    w.extend(xs);
    w
  }
}
