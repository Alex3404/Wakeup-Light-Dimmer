pub trait RoteryInterface {
    fn pressed(&self, pressed: bool);
    fn rotate_cw(&self, speed: u16);
    fn rotate_ccw(&self, speed: u16);
}
