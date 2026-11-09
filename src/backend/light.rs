/// Light around a character, in linear RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    /// colour of the surroundings, tints the shadows
    pub ambient: [f32; 3],
    /// colour of the brightest part of the screen, tints the lit side
    pub key: [f32; 3],
    /// direction to the key light on screen (x right, y up), length = how
    /// one-sided the light is
    pub dir: [f32; 2],
}

impl Default for Light {
    fn default() -> Self {
        Self { ambient: [0.5; 3], key: [1.0; 3], dir: [-0.45, 0.55] }
    }
}
