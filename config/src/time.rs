use std::time::{SystemTime, UNIX_EPOCH};

use crate::ast::{DataUnit, DelayCalculator, DistanceUnit, TimeUnit, TimestepConfig};
use crate::units::DecimalScaled;

impl TimestepConfig {
    /// Get the time elapsed from a specific point in the desired units
    pub fn time_from(&self, n: u64, unit: TimeUnit, start: &SystemTime) -> u64 {
        let duration = start.duration_since(UNIX_EPOCH).unwrap();
        let start = match unit {
            TimeUnit::Seconds => duration.as_secs(),
            TimeUnit::Milliseconds => duration.as_millis() as u64,
            TimeUnit::Microseconds => duration.as_micros() as u64,
            _ => unreachable!(),
        };
        start + self.elapsed(n, unit)
    }

    /// Provide the time since UNIX epoch in the requested unit.
    pub fn time(&self, n: u64, unit: TimeUnit) -> u64 {
        self.time_from(n, unit, &self.start)
    }

    /// Provide the time elapsed since simulation start.
    pub fn elapsed(&self, n: u64, unit: TimeUnit) -> u64 {
        let base = n / 10u64.pow(self.unit.power() as u32);
        let scalar = match unit {
            TimeUnit::Seconds => 1,
            TimeUnit::Milliseconds => 1_000,
            TimeUnit::Microseconds => 1_000_000,
            TimeUnit::Nanoseconds => 1_000_000_000,
            _ => unreachable!(),
        };
        base * scalar
    }
}

impl DelayCalculator {
    /// Determine how many timesteps are required to delay for based on the
    /// distance of the transmission and amount of data to transmit.
    ///
    /// Params:
    /// - `distance`: Distance in `distance_unit`s.
    /// - `amount`: Amount of data in `data_unit`s.
    ///
    /// Returns:
    /// - Number of timeseps to delay.
    pub fn timestep_delay(
        &self,
        distance: f64,
        amount: u64,
        data_unit: DataUnit,
        distance_unit: DistanceUnit,
    ) -> u64 {
        let (proc_num, proc_den) =
            Self::timesteps_required(amount, data_unit, self.processing, self.ts_config);
        let (trans_num, trans_den) =
            Self::timesteps_required(amount, data_unit, self.transmission, self.ts_config);
        let prop_timesteps = self.propagation_timesteps_f64(distance, distance_unit);
        let mut num = proc_num * trans_den + trans_num * proc_den;
        let den = proc_den * trans_den;
        // If this takes any time at all, make sure the numerator has something
        // so the event doesn't happen instantaneously.
        let added_timesteps = prop_timesteps * den as f64;
        if added_timesteps as u64 == 0 && added_timesteps > 0.0 {
            num += 1
        } else {
            num += (prop_timesteps * den as f64) as u64;
        }
        num.div_ceil(den)
    }

    pub fn processing_timesteps_u64(&self, amount: u64, data_unit: DataUnit) -> (u64, u64) {
        Self::timesteps_required(amount, data_unit, self.processing, self.ts_config)
    }

    pub fn transmission_timesteps_u64(&self, amount: u64, data_unit: DataUnit) -> (u64, u64) {
        Self::timesteps_required(amount, data_unit, self.transmission, self.ts_config)
    }

    pub fn processing_timesteps_f64(&self, amount: u64, data_unit: DataUnit) -> f64 {
        let (num, den) =
            Self::timesteps_required(amount, data_unit, self.processing, self.ts_config);
        num as f64 / den as f64
    }

    pub fn transmission_timesteps_f64(&self, amount: u64, data_unit: DataUnit) -> f64 {
        let (num, den) =
            Self::timesteps_required(amount, data_unit, self.transmission, self.ts_config);
        num as f64 / den as f64
    }

    pub fn propagation_timesteps_f64(&self, distance: f64, unit: DistanceUnit) -> f64 {
        // Get the distance into the desired units
        // Number of `distance_unit` / `time_unit` for value of `distance`
        let (should_scale_down, ratio) = DistanceUnit::ratio(self.propagation.distance, unit);
        // Scale distance units
        let scalar = 10u64
            .checked_pow(ratio.try_into().unwrap())
            .expect("Exponentiation overflow.") as f64;
        let distance = if should_scale_down {
            distance / scalar
        } else {
            distance * scalar
        };

        // Create the context for the expression to be evaluated in
        let mut ctx = meval::Context::new();
        ctx.var("distance", distance);
        ctx.var("d", distance);
        let time_units = self
            .propagation
            .rate
            .parse::<meval::Expr>()
            .expect("unable to parse meval expression")
            .eval_with_context(ctx)
            .expect("could not evaluate distance time var");

        // Scale time units
        let (should_scale_down, time_ratio) =
            TimeUnit::ratio(self.propagation.time, self.ts_config.unit);
        let scalar = 10_u64
            .checked_pow(time_ratio.try_into().unwrap())
            .expect("Exponentiation overflow.") as f64;
        if should_scale_down {
            time_units / scalar
        } else {
            time_units * scalar
        }
    }
}

impl Default for TimestepConfig {
    fn default() -> Self {
        Self {
            length: Self::DEFAULT_TIMESTEP_LEN,
            unit: TimeUnit::default(),
            count: Self::DEFAULT_TIMESTEP_COUNT,
            start: SystemTime::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use crate::ast::{DataRate, DataUnit, DelayCalculator, Delays, DistanceTimeVar, DistanceUnit};

    use super::*;

    use DataUnit::*;
    use DistanceUnit::*;

    #[test]
    fn delay_calculator() {
        let ts_config = TimestepConfig {
            length: NonZeroU64::new(1).unwrap(),
            unit: TimeUnit::Seconds,
            count: NonZeroU64::new(1000000).unwrap(),
            ..Default::default()
        };
        let transmission = DataRate {
            rate: 200,
            data: DataUnit::Bit,
            time: TimeUnit::Seconds,
        };
        let processing = DataRate {
            rate: 200,
            data: DataUnit::Bit,
            time: TimeUnit::Seconds,
        };
        let propagation = DistanceTimeVar {
            rate: "5 * d".parse().unwrap(),
            time: TimeUnit::Seconds,
            distance: DistanceUnit::Kilometers,
        };
        let delays = Delays {
            transmission,
            processing,
            propagation,
        };
        let mut calculator = DelayCalculator::validate(delays, ts_config).unwrap();
        let tests = [
            // Data unit conversions
            (0.0, 200, Byte, Kilometers, (2.0 * 8.0_f64).ceil() as u64),
            (
                0.0,
                200,
                Kilobit,
                Kilometers,
                (2.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Kilobyte,
                Kilometers,
                (2.0 * 8.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Megabit,
                Kilometers,
                (2.0 * 1024.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Megabyte,
                Kilometers,
                (2.0 * 8.0 * 1024.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Gigabit,
                Kilometers,
                (2.0 * 1024.0 * 1024.0 * 1024.0_f64).ceil() as u64,
            ),
            (
                0.0,
                200,
                Gigabyte,
                Kilometers,
                (2.0 * 8.0 * 1024.0 * 1024.0 * 1024.0_f64).ceil() as u64,
            ),
            // Distance conversions (propagation distances)
            (0.0, 0, Bit, Millimeters, 0),
            (0.001, 0, Bit, Millimeters, 1),
            (1.0, 0, Bit, Millimeters, 1),
            (100.0, 0, Bit, Millimeters, 1),
            (100.0 * 100.0, 0, Bit, Millimeters, 1),
            (100.0 * 100.0 * 99.0, 0, Bit, Millimeters, 5),
            (100.0 * 100.0 * 100.0, 0, Bit, Millimeters, 5),
            (100.0 * 100.0 * 200.0, 0, Bit, Millimeters, 10),
            (100.0 * 100.0 * 201.0, 0, Bit, Millimeters, 11),
            (100.0 * 100.0 * 300.0, 0, Bit, Millimeters, 15),
            (100.0 * 100.0 * 400.0, 0, Bit, Millimeters, 20),
            (100.0 * 100.0 * 400.0001, 0, Bit, Millimeters, 20),
            (100.0 * 100.0 * 1000.0, 0, Bit, Millimeters, 50),
            (100.0 * 100.0 * 1001.0, 0, Bit, Millimeters, 51),
            // Full pipeline (numerator/denominator conversions)
            (0.0001, 0, Bit, Kilometers, 1),
            (0.0, 1, Bit, Kilometers, 1),
            (0.0, 100, Bit, Kilometers, 1),
            (1.0, 0, Bit, Kilometers, 5),
            (1.0, 200, Bit, Kilometers, 7),
            (1.4, 200, Bit, Kilometers, 9),
            (1.9, 200, Bit, Kilometers, 12),
            (2.0, 200, Bit, Kilometers, 12),
            // Conversions on both units
            (
                0.0001,
                1,
                Kilobyte,
                Meters,
                (2.0 * 1024.0 * 8.0 / 200.0_f64).ceil() as u64,
            ),
            (
                1.0,
                1,
                Kilobyte,
                Meters,
                (2.0 * 1024.0 * 8.0 / 200.0_f64).ceil() as u64,
            ),
            (
                100.0,
                1,
                Kilobyte,
                Meters,
                (2.0 * 1024.0 * 8.0 / 200.0_f64).ceil() as u64 + 1,
            ),
            (
                1000.0,
                1,
                Kilobyte,
                Meters,
                (2.0 * 1024.0 * 8.0 / 200.0_f64).ceil() as u64 + 5,
            ),
        ];
        for (distance, amount, data_unit, distance_unit, expected) in tests {
            assert_eq!(
                calculator.timestep_delay(distance, amount, data_unit, distance_unit),
                expected
            );
        }

        // Test nonlinear expressions
        calculator.propagation = DistanceTimeVar {
            rate: "5 * d^2".parse().unwrap(),
            time: TimeUnit::Seconds,
            distance: DistanceUnit::Meters,
        };
        let tests = [
            // Distance conversions (propagation distances)
            (0.1, 0, Bit, Millimeters, 1),
            (1.0, 0, Bit, Millimeters, 1),
            (100.0, 0, Bit, Millimeters, 1),
            (10000.0, 0, Bit, Millimeters, 500),
            (0.1, 0, Bit, Centimeters, 1),
            (1.0, 0, Bit, Centimeters, 1),
            (100.0, 0, Bit, Centimeters, 5),
            (10000.0, 0, Bit, Centimeters, 50000),
            (0.1, 0, Bit, Meters, 1),
            (1.0, 0, Bit, Meters, 5),
            (100.0, 0, Bit, Meters, 50000),
            (10000.0, 0, Bit, Meters, 500000000),
            (0.1, 0, Bit, Kilometers, 50000),
            (1.0, 0, Bit, Kilometers, 5000000),
            (100.0, 0, Bit, Kilometers, 50000000000),
        ];
        for (distance, amount, data_unit, distance_unit, expected) in tests {
            assert_eq!(
                calculator.timestep_delay(distance, amount, data_unit, distance_unit),
                expected
            );
        }
    }
}
