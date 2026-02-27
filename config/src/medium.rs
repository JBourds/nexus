use std::f64::consts::PI;

use rand::Rng;

use crate::ast::{DistanceUnit, Medium, RssiProbExpr};

impl Medium {
    pub fn noise_floor_dbm(&self) -> f64 {
        match self {
            Medium::Wireless { rx_min_dbm, .. } => *rx_min_dbm,
            Medium::Wired { rx_min_dbm, .. } => *rx_min_dbm,
        }
    }

    pub fn tx_min_dbm(&self) -> f64 {
        match self {
            Medium::Wireless { tx_min_dbm, .. } => *tx_min_dbm,
            Medium::Wired { tx_min_dbm, .. } => *tx_min_dbm,
        }
    }

    pub fn tx_max_dbm(&self) -> f64 {
        match self {
            Medium::Wireless { tx_max_dbm, .. } => *tx_max_dbm,
            Medium::Wired { tx_max_dbm, .. } => *tx_max_dbm,
        }
    }

    pub fn rssi(&self, tx_power_dbm: f64, distance_meters: f64) -> f64 {
        match self {
            Medium::Wireless { .. } => self.rssi_wireless(tx_power_dbm, distance_meters),
            Medium::Wired { .. } => self.rssi_wired(tx_power_dbm, distance_meters),
        }
    }

    fn rssi_wireless(&self, tx_power_dbm: f64, distance_meters: f64) -> f64 {
        let Self::Wireless {
            wavelength_meters,
            gain,
            tx_min_dbm,
            tx_max_dbm,
            ..
        } = self
        else {
            unreachable!();
        };

        let tx_power_dbm = tx_power_dbm.clamp(*tx_min_dbm, *tx_max_dbm);

        if distance_meters <= f64::EPSILON {
            return tx_power_dbm + *gain;
        }

        let path_loss_db = 20.0 * (4.0 * PI * distance_meters / wavelength_meters).log10();

        tx_power_dbm + *gain - path_loss_db
    }

    fn rssi_wired(&self, tx_power_dbm: f64, distance_meters: f64) -> f64 {
        let Self::Wired {
            r,
            l,
            c,
            g,
            f,
            tx_min_dbm,
            tx_max_dbm,
            ..
        } = self
        else {
            unreachable!();
        };

        let tx_power_dbm = tx_power_dbm.clamp(*tx_min_dbm, *tx_max_dbm);

        if distance_meters <= f64::EPSILON {
            return tx_power_dbm;
        }

        let omega = 2.0 * PI * f;

        // Complex magnitude approximation for attenuation constant
        let r_term = r;
        let l_term = omega * l;
        let g_term = g;
        let c_term = omega * c;

        // Approximate alpha (Np/m)
        let alpha = 0.5
            * ((r_term * g_term + l_term * c_term)
                + ((r_term * g_term - l_term * c_term).powi(2)
                    + (r_term * c_term + l_term * g_term).powi(2))
                .sqrt())
            .sqrt();

        // Convert Nepers → dB
        let loss_db = 8.686 * alpha * distance_meters;

        tx_power_dbm - loss_db
    }
}

impl Default for Medium {
    fn default() -> Self {
        Self::Wired {
            rx_min_dbm: f64::MIN,
            tx_min_dbm: f64::MIN,
            tx_max_dbm: f64::MAX,
            r: 0.0,
            l: 0.0,
            c: 0.0,
            g: 0.0,
            f: f64::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::SignalShape;

    use super::*;

    #[test]
    fn wireless_rssi_zero_distance() {
        let medium = Medium::Wireless {
            shape: SignalShape::Omnidirectional,
            wavelength_meters: 0.125, // 2.4 GHz
            gain: 0.0,
            rx_min_dbm: -100.0,
            tx_min_dbm: -50.0,
            tx_max_dbm: 50.0,
        };

        let tx = 10.0;
        let rssi = medium.rssi(tx, 0.0);

        // At zero distance we defined RSSI == TX power
        assert_eq!(rssi, tx);
    }

    #[test]
    fn wireless_rssi_decreases_with_distance() {
        let medium = Medium::Wireless {
            shape: SignalShape::Omnidirectional,
            wavelength_meters: 0.125,
            gain: 0.0,
            rx_min_dbm: -100.0,
            tx_min_dbm: -50.0,
            tx_max_dbm: 50.0,
        };

        let tx = 0.0;

        let rssi_1m = medium.rssi(tx, 1.0);
        let rssi_10m = medium.rssi(tx, 10.0);
        let rssi_100m = medium.rssi(tx, 100.0);

        assert!(rssi_1m > rssi_10m);
        assert!(rssi_10m > rssi_100m);
    }

    #[test]
    fn wireless_inverse_square_law() {
        // Free space should drop ~6 dB per distance doubling
        let medium = Medium::Wireless {
            shape: SignalShape::Omnidirectional,
            wavelength_meters: 0.125,
            gain: 0.0,
            rx_min_dbm: -100.0,
            tx_min_dbm: -50.0,
            tx_max_dbm: 50.0,
        };

        let tx = 0.0;
        let distances: Vec<_> = (0..10).map(|i| (1 << i) as f64).collect();
        // Expect halving within 5% error
        let tolerance = 0.05;
        let mut prev = None;
        for d in distances {
            let rssi = medium.rssi(tx, d);
            if let Some(prev) = prev {
                let drop = prev - rssi;
                let expected: f64 = drop - 6.0;
                // Expect approximately 6 dB (within tolerance)
                assert!(expected.abs() < tolerance, "prev: {prev}, drop: {drop}");
            }
            prev = Some(rssi);
        }
    }

    #[test]
    fn wireless_tx_clamping() {
        let medium = Medium::Wireless {
            shape: SignalShape::Omnidirectional,
            wavelength_meters: 0.125,
            gain: 0.0,
            rx_min_dbm: -100.0,
            tx_min_dbm: -10.0,
            tx_max_dbm: 10.0,
        };

        let rssi = medium.rssi(100.0, 1.0);

        // Should clamp to 10 dBm before computing path loss
        let expected = medium.rssi(10.0, 1.0);

        assert_eq!(rssi, expected);
    }

    #[test]
    fn wired_rssi_zero_distance() {
        let medium = Medium::Wired {
            rx_min_dbm: -100.0,
            tx_min_dbm: -50.0,
            tx_max_dbm: 50.0,
            r: 0.1,
            l: 1e-6,
            c: 1e-12,
            g: 0.01,
            f: 1e6,
        };

        let tx = 5.0;
        let rssi = medium.rssi(tx, 0.0);

        assert_eq!(rssi, tx);
    }

    #[test]
    fn wired_rssi_decreases_with_distance() {
        let medium = Medium::Wired {
            rx_min_dbm: -100.0,
            tx_min_dbm: -50.0,
            tx_max_dbm: 50.0,
            r: 0.1,
            l: 1e-6,
            c: 1e-12,
            g: 0.01,
            f: 1e6,
        };

        let tx = 0.0;

        let rssi_1m = medium.rssi(tx, 1.0);
        let rssi_10m = medium.rssi(tx, 10.0);

        assert!(rssi_1m > rssi_10m);
    }

    #[test]
    fn wired_loss_scales_with_distance() {
        let medium = Medium::Wired {
            rx_min_dbm: -100.0,
            tx_min_dbm: -50.0,
            tx_max_dbm: 50.0,
            r: 0.1,
            l: 1e-6,
            c: 1e-12,
            g: 0.01,
            f: 1e6,
        };

        let tx = 0.0;

        let rssi_1m = medium.rssi(tx, 1.0);
        let rssi_2m = medium.rssi(tx, 2.0);

        let loss_1m = tx - rssi_1m;
        let loss_2m = tx - rssi_2m;

        // Loss should roughly double when distance doubles
        assert!(((loss_2m / loss_1m - 2.0) as f64).abs() < 0.2);
    }

    #[test]
    fn wired_frequency_increases_loss() {
        let base = Medium::Wired {
            rx_min_dbm: -100.0,
            tx_min_dbm: -50.0,
            tx_max_dbm: 50.0,
            r: 0.1,
            l: 1e-6,
            c: 1e-12,
            g: 0.01,
            f: 1e5,
        };

        let high_freq = Medium::Wired {
            f: 1e9,
            // same as in `base`
            rx_min_dbm: -100.0,
            tx_min_dbm: -50.0,
            tx_max_dbm: 50.0,
            r: 0.1,
            l: 1e-6,
            c: 1e-12,
            g: 0.01,
        };

        let tx = 0.0;

        let low_loss = base.rssi(tx, 10.0);
        let high_loss = high_freq.rssi(tx, 10.0);

        // Higher frequency should attenuate more
        assert!(high_loss < low_loss);
    }
}

impl RssiProbExpr {
    pub fn rssi(
        &self,
        tx_power_dbm: f64,
        distance: f64,
        unit: DistanceUnit,
        medium: &Medium,
    ) -> f64 {
        let (scale_up, scale) = DistanceUnit::ratio(unit, DistanceUnit::Meters);
        let scalar = 10u64.pow(scale as u32) as f64;
        let distance_meters = if scale_up {
            distance * scalar
        } else {
            distance / scalar
        };
        medium.rssi(tx_power_dbm, distance_meters)
    }

    /// Sample as `sample_rssi` but calculates rssi (for one shot)
    pub fn sample_oneshot(
        &self,
        tx_power_dbm: f64,
        distance: f64,
        unit: DistanceUnit,
        medium: &Medium,
        rng: &mut rand::rngs::StdRng,
    ) -> bool {
        let rssi = self.rssi(tx_power_dbm, distance, unit, medium);
        self.probability(rssi) > rng.random_range(0.0..=1.0)
    }

    /// Returns `true` if the event happens, and `false` otherwise
    pub fn sample_rssi(&self, rssi: f64, rng: &mut rand::rngs::StdRng) -> bool {
        self.probability(rssi) > rng.random_range(0.0..=1.0)
    }

    /// # Safety
    /// This function is entirely safe to use and will never cause major issues.
    /// If the value for `prob` is not properly constrained from 0.0 - 1.0 this
    /// will give bogus results though.
    pub unsafe fn sample_unchecked(&self, prob: f64, rng: &mut rand::rngs::StdRng) -> bool {
        prob > rng.random_range(0.0..=1.0)
    }

    pub fn probability(&self, rssi: f64) -> f64 {
        let mut ctx = meval::Context::new();
        ctx.var("rssi", rssi);
        ctx.var("snr", rssi - self.noise_floor_dbm);
        self.expr
            .parse::<meval::Expr>()
            .expect("this gets checked in validation")
            .eval_with_context(ctx)
            .expect("couldn't evaluate expression")
            .clamp(0.0, 1.0)
    }
}

impl Default for RssiProbExpr {
    fn default() -> Self {
        Self {
            expr: "0".parse().unwrap(),
            noise_floor_dbm: f64::MIN,
        }
    }
}
