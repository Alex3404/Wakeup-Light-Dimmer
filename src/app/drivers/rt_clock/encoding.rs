pub trait Encoding {
    fn decode(byte: u8) -> u8;
    fn encode(val: u8) -> u8;
}

pub struct Bcd<const TENS: u8, const ONES: u8>;
impl<const TENS: u8, const ONES: u8> Bcd<TENS, ONES> {
    const TENS_MASK: u8 = (1 << TENS) - 1;
    const ONES_MASK: u8 = (1 << ONES) - 1;
}

impl<const TENS: u8, const ONES: u8> Encoding for Bcd<TENS, ONES> {
    fn decode(b: u8) -> u8 {
        ((b >> ONES) * 10) & Self::TENS_MASK + (b & Self::ONES_MASK)
    }

    fn encode(v: u8) -> u8 {
        ((v / 10) << ONES) & Self::TENS_MASK | (v % 10) & Self::ONES_MASK
    }
}
