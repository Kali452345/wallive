//! Which video comes next when several take turns.

/// Tray choices for the time between videos, in minutes.
pub const SWITCH_CHOICES: [u32; 5] = [1, 5, 15, 30, 60];

/// Index of the video to show after `current`, among those marked `ready`
/// (imported and present). Never `current` itself; `None` if no other video
/// is ready. In order: the next ready one, wrapping around. Shuffled: a
/// random ready one, `random` choosing among them.
pub fn next(current: usize, ready: &[bool], shuffle: bool, random: u64) -> Option<usize> {
    let n = ready.len();
    let others: Vec<usize> = (1..n)
        .map(|k| (current + k) % n)
        .filter(|&i| ready[i])
        .collect();
    if others.is_empty() {
        return None;
    }
    Some(if shuffle {
        others[(random % others.len() as u64) as usize]
    } else {
        others[0]
    })
}

/// Small xorshift generator for shuffling; not for anything secret.
pub struct Rng(u64);

impl Rng {
    pub fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Self((nanos ^ u64::from(std::process::id()).rotate_left(32)) | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_order_wraps_and_skips_unready() {
        let all = [true, true, true];
        assert_eq!(next(0, &all, false, 0), Some(1));
        assert_eq!(next(2, &all, false, 0), Some(0));
        assert_eq!(next(0, &[true, false, true], false, 0), Some(2));
        // The current video need not be ready itself (its import may have
        // been replaced); the next ready one still follows it.
        assert_eq!(next(1, &[true, false, false], false, 0), Some(0));
    }

    #[test]
    fn nothing_else_ready() {
        assert_eq!(next(0, &[true], false, 0), None);
        assert_eq!(next(0, &[true, false, false], true, 7), None);
        assert_eq!(next(0, &[], false, 0), None);
    }

    #[test]
    fn shuffle_never_repeats_and_reaches_all() {
        let ready = [true; 4];
        let mut rng = Rng(12345);
        let mut seen = [false; 4];
        let mut current = 0;
        for _ in 0..200 {
            let n = next(current, &ready, true, rng.next()).unwrap();
            assert_ne!(n, current);
            seen[n] = true;
            current = n;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn rng_is_not_stuck() {
        let mut rng = Rng::seeded();
        let a = rng.next();
        assert_ne!(a, rng.next());
    }
}
