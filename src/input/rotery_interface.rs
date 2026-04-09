use embassy_executor::Spawner;

pub trait RoteryInterface {
    fn pressed(&mut self, pressed: bool, spawner: Spawner);
    fn rotate_cw(&mut self, spawner: Spawner);
    fn rotate_ccw(&mut self, spawner: Spawner);
}
