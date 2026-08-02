use std::f32::consts::TAU as τ;

pub fn standard(t: f32) -> [u8; 3] {
  // https://people.phy.cam.ac.uk/dag9/CUBEHELIX/cubewedges.html
  custom(t, [0.5, -1.5, 1., 1.])
}

pub fn custom(t: f32, [start, rot, sat, gamma]: [f32; 4]) -> [u8; 3] {
  let t = t.clamp(0., 1.).powf(gamma);
  hsl([(start - 1.) / 3. + rot * t, sat, t])
}

pub fn split(x: f32, sign: f32) -> [u8; 3] {
  let x = x.clamp(-1., 1.);
  hsl([(x + 1.) / -3., 1., 0.5 + 0.5 * sign * (1. - x.abs())])
}

pub fn hsl([h, s, l]: [f32; 3]) -> [u8; 3] {
  let s = s * l * (1. - l) / 2.;
  let (sin, cos) = h.mul_add(τ, τ / 3.).sin_cos();
  coefficients().map(|[kc, ks]| {
    let n = kc.mul_add(cos, ks * sin).mul_add(s, l).mul_add(255., 0.5);
    unsafe { n.to_int_unchecked() }
  })
}

fn coefficients() -> [[f32; 2]; 3] {
  let [r, g, b]: [f32; _] = [0.2126, 0.7152, 0.0722]; // [0.30, 0.59, 0.11]
  let ks = 2. / (r * r + g * g).sqrt();
  let kc = ks / (r * r + g * g + b * b).sqrt();
  [[-kc * r * b, ks * g], [-kc * g * b, -ks * r], [kc * (r * r + g * g), 0.]]
}
