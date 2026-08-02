#![allow(dead_code)]

pub fn black(x: f32) -> [u8; 3] {
  let x = x.clamp(-1., 1.);
  let t = x * x; // let t = x.abs();
  quantize(if x < 0. { [t, 0., 0.] } else { [0., t, 0.] })
}

pub fn white(x: f32) -> [u8; 3] {
  let x = x.clamp(-1., 1.);
  let t = 1. - x * x; // let t = 1. - x.abs();
  quantize(if x < 0. { [1., t, t] } else { [t, 1., t] })
}

pub fn quantize(rgb: [f32; 3]) -> [u8; 3] {
  rgb.map(|c| c.mul_add(255., 0.5) as u8)
}

pub fn mix(t: f32, a: [f32; 3], b: [f32; 3]) -> [u8; 3] {
  quantize(std::array::from_fn(|i| (1. - t) * a[i] + t * b[i]))
}

pub fn split(t: f32, base: [f32; 3], neg: [f32; 3], pos: [f32; 3]) -> [u8; 3] {
  mix(t * t, base, if t < 0. { neg } else { pos })
}
