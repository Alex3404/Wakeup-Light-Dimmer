pub trait RoteryInterface {
    fn pressed(&self, pressed: bool);
    fn rotate_cw(&self);
    fn rotate_ccw(&self);
}
