pub trait RoteryInterface {
    fn pressed(&mut self, pressed: bool);
    fn rotate_cw(&mut self);
    fn rotate_ccw(&mut self);
}
