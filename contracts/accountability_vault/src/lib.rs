use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct Vault {
    // staked amount in smallest units
    staked: AtomicU64,
    // single-spend guard: once a withdrawal or slash succeeds, this becomes true
    spent: AtomicBool,
    // whether the vault was slashed
    slashed: AtomicBool,
    // deadline as unix seconds
    deadline_secs: u64,
}

impl Vault {
    pub fn new(staked: u64, deadline_secs: u64) -> Arc<Self> {
        Arc::new(Vault {
            staked: AtomicU64::new(staked),
            spent: AtomicBool::new(false),
            slashed: AtomicBool::new(false),
            deadline_secs,
        })
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_secs()
    }

    // Attempt to withdraw before or at the deadline. Returns the paid out amount on success.
    pub fn withdraw(&self, now_secs: Option<u64>) -> Result<u64, &'static str> {
        let now = now_secs.unwrap_or_else(Self::now_secs);
        // Guard: cannot withdraw after deadline
        if now > self.deadline_secs {
            return Err("deadline_passed");
        }

        // Ensure we only pay out once (single-spend)
        let was_spent =
            self.spent
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
        match was_spent {
            Ok(false) | Ok(_) => {
                // take the stake
                let amount = self.staked.swap(0, Ordering::SeqCst);
                Ok(amount)
            }
            Err(_) => Err("already_spent"),
        }
    }

    // Slash the vault after the deadline. Returns slashed amount on success.
    pub fn slash_on_miss(&self, now_secs: Option<u64>) -> Result<u64, &'static str> {
        let now = now_secs.unwrap_or_else(Self::now_secs);
        if now <= self.deadline_secs {
            return Err("deadline_not_passed");
        }

        // Ensure single-spend: only one of withdraw/slash can succeed
        let was_spent =
            self.spent
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
        match was_spent {
            Ok(false) | Ok(_) => {
                let amount = self.staked.swap(0, Ordering::SeqCst);
                self.slashed.store(true, Ordering::SeqCst);
                Ok(amount)
            }
            Err(_) => Err("already_spent"),
        }
    }

    pub fn is_slashed(&self) -> bool {
        self.slashed.load(Ordering::SeqCst)
    }

    pub fn remaining(&self) -> u64 {
        self.staked.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::Barrier;
    use std::thread;

    #[test]
    fn withdraw_before_deadline_succeeds() {
        let now = Vault::now_secs();
        let vault = Vault::new(1_000, now + 60);
        let res = vault.withdraw(Some(now));
        assert_eq!(res.unwrap(), 1_000);
        assert_eq!(vault.remaining(), 0);
        assert!(!vault.is_slashed());
    }

    #[test]
    fn withdraw_after_deadline_fails() {
        let now = Vault::now_secs();
        let vault = Vault::new(2_000, now - 1);
        let res = vault.withdraw(Some(now));
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "deadline_passed");
        assert_eq!(vault.remaining(), 2_000);
    }

    #[test]
    fn slash_after_deadline_succeeds() {
        let now = Vault::now_secs();
        let vault = Vault::new(3_000, now - 10);
        let res = vault.slash_on_miss(Some(now));
        assert_eq!(res.unwrap(), 3_000);
        assert!(vault.is_slashed());
        assert_eq!(vault.remaining(), 0);
    }

    #[test]
    fn withdraw_vs_slash_race() {
        // Simulate a race where one thread calls withdraw and another calls slash_on_miss
        // Use barrier to start simultaneously and sample times so one path should fail.
        let now = Vault::now_secs();
        // set deadline to "now" so that timing is tight; both callers supply now or now+1
        let deadline = now;
        let vault = Vault::new(10_000, deadline);

        // Shared counters for payouts
        let withdraw_paid = Arc::new(AtomicU64::new(0));
        let slash_paid = Arc::new(AtomicU64::new(0));

        let b = Arc::new(Barrier::new(3));

        let v1 = vault.clone();
        let wpaid = withdraw_paid.clone();
        let b1 = b.clone();
        let t1 = thread::spawn(move || {
            b1.wait();
            // try withdraw with now (deadline == now) — allowed
            match v1.withdraw(Some(deadline)) {
                Ok(a) => wpaid.store(a, Ordering::SeqCst),
                Err(_) => (),
            }
        });

        let v2 = vault.clone();
        let spaid = slash_paid.clone();
        let b2 = b.clone();
        let t2 = thread::spawn(move || {
            b2.wait();
            // try slash with now+1 (deadline passed)
            match v2.slash_on_miss(Some(deadline + 1)) {
                Ok(a) => spaid.store(a, Ordering::SeqCst),
                Err(_) => (),
            }
        });

        // let threads start
        b.wait();
        t1.join().unwrap();
        t2.join().unwrap();

        let w = withdraw_paid.load(Ordering::SeqCst);
        let s = slash_paid.load(Ordering::SeqCst);

        // Ensure single-spend: only one of w or s is non-zero and total equals original
        assert!((w == 10_000 && s == 0) || (w == 0 && s == 10_000));
        assert_eq!(w + s, 10_000);
        assert_eq!(vault.remaining(), 0);
    }
}
