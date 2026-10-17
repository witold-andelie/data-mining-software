//! The random numbers of Modeler's seeded record nodes: the `java.util.Random` generator (a
//! 48-bit linear congruential generator, from its public specification), seeded with the node's
//! seed and drawn once per record in input order (`oracle/random/*`,
//! `xtask/oracle/probes/random_streams.py`):
//! - Partition: `next_int(100)` per record; the record goes to the first partition whose
//!   cumulative percentage exceeds the draw (`part_default`, `part_seed0`/`1`/`2`, `part_30_70`,
//!   `part_3way`: 1000/1000 records each; `part_n10`, `part_n100` are prefixes of the same draws).
//! - Sample, random percentage: `next_double() * 100 < percent` per record (`sample_rand50`,
//!   `sample_rand20_seed1`: the same 505 and 213 records).
//! - Balance has no seed: two Modeler runs of `balance_boost` and `balance_reduce` keep different
//!   records, so its draws cannot be reproduced; we use this generator with a fixed seed.

const MULTIPLIER: u64 = 0x5_DEEC_E66D;
const MASK: u64 = (1 << 48) - 1;

pub(crate) struct JavaRandom {
    state: u64,
}

impl JavaRandom {
    pub(crate) fn new(seed: i64) -> JavaRandom {
        JavaRandom {
            state: (seed as u64 ^ MULTIPLIER) & MASK,
        }
    }

    /// The next `bits` (at most 32) random bits.
    fn next(&mut self, bits: u32) -> u32 {
        self.state = self.state.wrapping_mul(MULTIPLIER).wrapping_add(0xB) & MASK;
        (self.state >> (48 - bits)) as u32
    }

    /// Uniform in `0..bound` (`bound` > 0), with Java's rejection of the biased top range.
    pub(crate) fn next_int(&mut self, bound: u32) -> u32 {
        if bound.is_power_of_two() {
            return ((u64::from(bound) * u64::from(self.next(31))) >> 31) as u32;
        }
        loop {
            let bits = self.next(31);
            let value = bits % bound;
            if u64::from(bits - value) + u64::from(bound - 1) < (1 << 31) {
                return value;
            }
        }
    }

    /// Uniform in [0, 1) with 53 random bits.
    pub(crate) fn next_double(&mut self) -> f64 {
        let high = u64::from(self.next(26));
        let low = u64::from(self.next(27));
        ((high << 27) + low) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::JavaRandom;

    #[test]
    fn partition_draws_of_the_default_seed() {
        // Seed 1234567: 46, 36, 51, 98, 48, so a 50/50 partition gives 1 1 2 2 1, the first five
        // records of `oracle/random/part_default`.
        let mut random = JavaRandom::new(1_234_567);
        let draws: Vec<u32> = (0..5).map(|_| random.next_int(100)).collect();
        assert_eq!(draws, [46, 36, 51, 98, 48]);
    }
}
