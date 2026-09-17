pub fn avg(a: i16, b: i16) -> i16 {
    return ((a as i32 + b as i32) / 2) as i16;
}

pub fn lerp(a: i16, b: i16, p: f32) -> i16 {
     p.mul_add(b as f32 - a as f32, a as f32) as i16
}
