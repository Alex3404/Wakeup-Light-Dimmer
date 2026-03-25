use embassy_time::Duration;

pub struct TimeRollingAverage<const N: usize> {
    sum: Duration,
    count: usize,
    buffer: [Duration; N],
    index: usize,
}

impl<const N: usize> TimeRollingAverage<N> {
    pub const fn new() -> Self {
        Self {
            sum: Duration::MIN,
            count: 0,
            buffer: [Duration::MIN; N],
            index: 0,
        }
    }

    pub fn new_sample(&mut self, value: Duration) -> Duration {
        let Some(sum) = self
            .sum
            .checked_sub(self.buffer[self.index])
            .and_then(|x: Duration| x.checked_add(value))
        else {
            return self.average();
        };

        self.sum = sum;
        self.buffer[self.index] = value;
        self.index = (self.index + 1) % N;

        if self.count < N {
            self.count += 1;
        }

        self.average()
    }

    pub fn average(&self) -> Duration {
        if self.count == 0 {
            return Duration::MIN;
        }

        let avg_micros = self.sum.as_micros() / self.count as u64;
        Duration::from_micros(avg_micros)
    }
}
