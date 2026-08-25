use fixed::types::I24F8;

pub trait RoteryInterface {
    fn pressed(&self, pressed: bool);
    fn rotate_cw(&self, rpm: I24F8);
    fn rotate_ccw(&self, rpm: I24F8);
}
