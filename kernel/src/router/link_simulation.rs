use config::units::DecimalScaled;

use super::*;
use crate::types::Node;
use std::collections::HashMap;

/// Deterministic per-pair-per-channel link parameters. Computed once for
/// pairs where both endpoints are still Static, then served from cache.
/// Random samples (packet drop, bit flips) still happen per call; only the
/// distance-dependent RSSI/SNR/probability calculations are reused.
#[derive(Clone, Debug)]
pub(super) struct CachedLink {
    pub distance: f64,
    pub distance_unit: DistanceUnit,
    /// RSSI as seen by the packet-loss model.
    pub pl_rssi: f64,
    /// Pre-evaluated packet-loss probability at `pl_rssi`. The
    /// `RssiProbExpr::probability` call evaluates a `meval` expression and
    /// is the most expensive thing in the link sim hot path.
    pub pl_prob: f64,
    /// RSSI as seen by the bit-error model (may differ from `pl_rssi`).
    pub be_rssi: f64,
    /// SNR derived from `be_rssi` and the medium's noise floor.
    pub be_snr: f64,
    /// Pre-evaluated bit-error probability at `be_rssi`.
    pub be_prob: f64,
    /// True when `pl_rssi < medium.noise_floor_dbm()`; signals "would
    /// always drop." Lets the cached send path skip re-comparing on every
    /// call.
    pub below_noise_floor: bool,
}

impl RoutingServer {
    /// Compute deterministic link parameters for the given pair on the
    /// given channel. Pure function; runs the path-loss math and the
    /// `meval` probability expressions.
    pub(super) fn compute_link(channel: &Channel, src: &Node, dst: &Node) -> CachedLink {
        let (distance, distance_unit) =
            config::ast::Position::distance(&src.position, &dst.position);
        let medium = &channel.link.medium;
        let tx_dbm = medium.tx_max_dbm();
        let pl_rssi = channel
            .link
            .packet_loss
            .rssi(tx_dbm, distance, distance_unit, medium);
        let be_rssi = channel
            .link
            .bit_error
            .rssi(tx_dbm, distance, distance_unit, medium);
        let be_snr = be_rssi - medium.noise_floor_dbm();
        let below_noise_floor = pl_rssi < medium.noise_floor_dbm();
        let pl_prob = if below_noise_floor {
            1.0
        } else {
            channel.link.packet_loss.probability(pl_rssi)
        };
        let be_prob = channel.link.bit_error.probability(be_rssi);
        CachedLink {
            distance,
            distance_unit,
            pl_rssi,
            pl_prob,
            be_rssi,
            be_snr,
            be_prob,
            below_noise_floor,
        }
    }

    /// Look up cached link parameters or compute and insert. Pairs where
    /// either endpoint is dynamic always recompute and never touch the
    /// cache.
    pub(super) fn lookup_or_compute_link(
        cache: &mut HashMap<(usize, usize, usize), CachedLink>,
        nodes: &[Node],
        channels: &[Channel],
        src_idx: usize,
        dst_idx: usize,
        channel_idx: usize,
    ) -> CachedLink {
        let src = &nodes[src_idx];
        let dst = &nodes[dst_idx];
        let channel = &channels[channel_idx];
        if src.is_dynamic || dst.is_dynamic {
            return Self::compute_link(channel, src, dst);
        }
        let key = (src_idx, dst_idx, channel_idx);
        if let Some(cached) = cache.get(&key) {
            return cached.clone();
        }
        let link = Self::compute_link(channel, src, dst);
        cache.insert(key, link.clone());
        link
    }

    /// Cached counterpart to `send_through_channel`: skips the RSSI
    /// recompute and the `meval` probability evaluation, keeping only the
    /// random sampling.
    pub(super) fn send_through_channel_cached<'a>(
        channel: &Channel,
        mut buf: Cow<'a, [u8]>,
        link: &CachedLink,
        rng: &mut StdRng,
    ) -> Option<(Cow<'a, [u8]>, bool, f64, f64)> {
        if link.below_noise_floor {
            warn!("Packet dropped (rssi = {})", link.pl_rssi);
            return None;
        }
        if channel.link.packet_loss.sample_unchecked(link.pl_prob, rng) {
            warn!(
                "Packet dropped (packet_loss, rssi = {})",
                link.pl_rssi
            );
            return None;
        }
        let mut had_bit_errors = false;
        if link.be_prob != 0.0 {
            let flips = (0..buf.len() * usize::try_from(u8::BITS).unwrap())
                .map(|_| channel.link.bit_error.sample_unchecked(link.be_prob, rng));
            let (_, flipped) = flip_bits(buf.to_mut(), flips);
            had_bit_errors = flipped > 0;
        }
        Some((buf, had_bit_errors, link.be_rssi, link.be_snr))
    }
}

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

}
