use embassy_time::Duration;

#[derive(Debug)]
pub struct TimeRollingAverage<const N: usize> {
    sum: Duration,
    count: usize,
    buffer: [Duration; N],
    index: usize,
}

impl<const N: usize> Default for TimeRollingAverage<N> {
    fn default() -> Self {
        Self {
            sum: Duration::MIN,
            count: 0,
            buffer: [Duration::MIN; N],
            index: 0,
        }
    }
}

impl<const N: usize> TimeRollingAverage<N> {
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
        self.index = (self.index.saturating_add(1)).checked_rem(N).unwrap_or(0);

        if self.count < N {
            self.count = self.count.saturating_add(1);
        }

        self.average()
    }

    pub fn average(&self) -> Duration {
        if self.count == 0 {
            return Duration::MIN;
        }

        let avg_micros = self.sum.as_micros().checked_div(self.count as u64).unwrap_or(0);
        Duration::from_micros(avg_micros)
    }

    pub fn is_full(&self) -> bool {
        self.count == N
    }
}
