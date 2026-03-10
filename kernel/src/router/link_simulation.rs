use config::ast::Link;

use super::*;

impl RoutingServer {
    /// Calculate the timesteps at which the message should be moved to its
    /// destination and, optionally (if ttl is specified), its expiration.
    pub(super) fn message_timesteps(
        channel: &Channel,
        sz: u64,
        ts_config: TimestepConfig,
        timestep: u64,
        distance: f64,
        distance_unit: DistanceUnit,
    ) -> (Timestep, Option<NonZeroU64>) {
        let unit = DataUnit::Byte;
        let delays = &channel.link.delays;
        let becomes_active_at = timestep
            + delays.transmission_timesteps_f64(sz, unit).round() as u64
            + delays
                .propagation_timesteps_f64(distance, distance_unit)
                .round() as u64
            + delays.processing_timesteps_f64(sz, unit).round() as u64;
        let expiration = channel.r#type.ttl().map(|ttl| {
            let (scale_down, ratio) = TimeUnit::ratio(channel.r#type.time_units(), ts_config.unit);
            let scalar = 10u64
                .checked_pow(ratio.try_into().unwrap())
                .expect("Exponentiation overflow.");
            let mut scaled_ttl = if scale_down {
                ttl.get().saturating_div(scalar)
            } else {
                ttl.get().saturating_mul(scalar)
            };

            // TODO: Better way to do this without all the divisions?
            let remaining =
                ts_config.length.get() - becomes_active_at.rem_euclid(ts_config.length.get());
            let mut expiration = becomes_active_at;
            if scaled_ttl >= remaining {
                expiration += 1;
                scaled_ttl -= remaining;
            }
            expiration += scaled_ttl / ts_config.length.get();
            NonZeroU64::new(expiration).unwrap()
        });
        (becomes_active_at, expiration)
    }

    /// Perform link simulation for:
    /// - dropped packets
    /// - bit errors
    pub(super) fn send_through_channel<'a>(
        channel: &Channel,
        mut buf: Cow<'a, [u8]>,
        distance: f64,
        unit: DistanceUnit,
        rng: &mut StdRng,
    ) -> Option<Cow<'a, [u8]>> {
        let Link {
            medium,
            packet_loss,
            bit_error,
            ..
        } = &channel.link;
        // TODO: Allow TX power to be configured via control file for the link
        let tx_dbm = medium.tx_max_dbm();
        let rssi = packet_loss.rssi(tx_dbm, distance, unit, medium);
        if rssi < medium.noise_floor_dbm() {
            warn!("Packet dropped (rssi = {rssi})");
            return None;
        }
        if packet_loss.sample_oneshot(tx_dbm, distance, unit, medium, rng) {
            warn!("Packet dropped (packet_loss, rssi = {rssi})");
            return None;
        }

        let rssi = bit_error.rssi(tx_dbm, distance, unit, medium);
        let ber = bit_error.probability(rssi);
        if ber != 0.0 {
            let flips = (0..buf.len() * usize::try_from(u8::BITS).unwrap())
                .map(|_| bit_error.sample_unchecked(ber, rng));
            let _ = flip_bits(buf.to_mut(), flips);
        }
        Some(buf)
    }
}
