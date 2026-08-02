use std::array;

pub fn header(text: &str) {
  println!();
  println!("\x1b[30m█{:▀>109}█\x1b[39m", "");
  println!("\x1b[30m█\x1b[39m  {text:105}  \x1b[30m█\x1b[39m");
  println!("\x1b[30m█{:▄>109}█\x1b[39m", "");
  println!();
}

pub fn windows<const N: usize, T>(slice: &[T], n: usize) -> [&[T]; N] {
  assert!(N * n <= slice.len());
  array::from_fn(|i| {
    let i = i * (slice.len() - n) / (N - 1);
    unsafe { slice.get_unchecked(i..i + n) }
  })
}

pub const fn p(p: f64) -> f64 {
  match p {
    0.8 => 1.281551565545,
    0.9 => 1.644853626951,
    0.95 => 1.959963984540,
    0.98 => 2.326347874041,
    0.99 => 2.575829303549,
    0.995 => 2.807033768344,
    0.998 => 3.090232306168,
    0.999 => 3.290526731492,
    0.9999 => 3.890591886413,
    0.99999 => 4.417173413469,
    0.999999 => 4.891638475699,
    0.9999999 => 5.326723886384,
    0.99999999 => 5.730728868236,
    0.999999999 => 6.109410204869,
    _ => unimplemented!(),
  }
}
