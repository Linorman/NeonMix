//! Room-owned AirPlay reservations. All calls occur in the Engine transaction.
use neonmix_airplay_adapter::Context;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub(super) const MULTI_CAPACITY: usize = 4;
const RESERVATION_TTL: Duration = Duration::from_secs(5);
const MAX_WIRE_ID: u64 = (1 << 53) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Owner {
    pub receiver: Uuid,
    pub generation: u64,
    pub connection: u64,
    pub request: u64,
    pub source: String,
}
#[derive(Clone)]
pub(super) struct Claim {
    pub owner: Owner,
    pub lane: usize,
    pub context: Context,
    pub deadline: Instant,
    pub active: bool,
}
#[derive(Default)]
pub(super) struct Admissions {
    pub claims: BTreeMap<Uuid, Claim>,
    next_id: u64,
}
impl Admissions {
    pub fn reserve(
        &mut self,
        owner: Owner,
        native_count: usize,
        multi: bool,
        free_lane: Option<usize>,
        native_streams: &[u64],
        now: Instant,
    ) -> Result<Claim, &'static str> {
        // Expired claims remain owned until their worker closes its media gate and
        // returns the producer. Never reuse a lane just because its lease elapsed.
        if self.claims.contains_key(&owner.receiver) {
            return Err("receiver_busy");
        }
        if self.claims.values().any(|c| c.owner.source == owner.source) {
            return Err("source_already_active");
        }
        if (multi && native_count + self.claims.len() >= MULTI_CAPACITY)
            || (!multi && (native_count > 1 || !self.claims.is_empty()))
        {
            return Err("room_capacity_full");
        }
        let lane = free_lane.ok_or("room_capacity_full")?;
        let session_id = self.allocate_id(native_streams)?;
        let stream_id = self.allocate_id(native_streams)?;
        let claim = Claim {
            owner,
            lane,
            context: Context {
                session_id,
                stream_id,
                stream_epoch: 1,
                format_epoch: 1,
                mapping_id: 1,
            },
            deadline: now + RESERVATION_TTL,
            active: false,
        };
        self.claims.insert(claim.owner.receiver, claim.clone());
        Ok(claim)
    }
    fn allocate_id(&mut self, native: &[u64]) -> Result<u64, &'static str> {
        loop {
            self.next_id = self
                .next_id
                .checked_add(1)
                .filter(|id| *id <= MAX_WIRE_ID)
                .ok_or("media_id_exhausted")?;
            let id = self.next_id;
            if !native.contains(&id)
                && !self
                    .claims
                    .values()
                    .any(|c| c.context.session_id == id || c.context.stream_id == id)
            {
                return Ok(id);
            }
        }
    }
    pub fn commit(&mut self, owner: &Owner, now: Instant) -> Result<Claim, &'static str> {
        let c = self
            .claims
            .get_mut(&owner.receiver)
            .ok_or("session_changed")?;
        if c.owner != *owner {
            return Err("session_changed");
        }
        if !c.active && now >= c.deadline {
            return Err("reservation_expired");
        }
        c.active = true;
        Ok(c.clone())
    }
    pub fn release(&mut self, owner: &Owner) -> Option<Claim> {
        if self
            .claims
            .get(&owner.receiver)
            .is_some_and(|c| c.owner == *owner)
        {
            self.claims.remove(&owner.receiver)
        } else {
            None
        }
    }
    pub fn native_capacity(&self, native: usize, multi: bool) -> bool {
        if multi {
            native + self.claims.len() <= MULTI_CAPACITY
        } else {
            self.claims.is_empty() || native <= 1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(receiver: Uuid, source: &str) -> Owner {
        Owner {
            receiver,
            generation: 1,
            connection: 1,
            request: 1,
            source: source.into(),
        }
    }
    #[test]
    fn reservations_exclude_duplicates_and_native_overcommit() {
        let mut a = Admissions::default();
        let now = Instant::now();
        let one = owner(Uuid::new_v4(), "a");
        let claim = a
            .reserve(one.clone(), 3, true, Some(7), &[1, 2], now)
            .unwrap();
        assert!(claim.context.session_id > 2);
        assert!(!a.native_capacity(4, true));
        assert_eq!(
            a.reserve(owner(Uuid::new_v4(), "b"), 3, true, Some(8), &[], now)
                .err(),
            Some("room_capacity_full")
        );
        assert_eq!(
            a.reserve(owner(Uuid::new_v4(), "a"), 0, true, Some(8), &[], now)
                .err(),
            Some("source_already_active")
        );
        let mut stale = one.clone();
        stale.generation += 1;
        assert!(a.release(&stale).is_none());
        assert!(a.commit(&one, now + RESERVATION_TTL).is_err());
        assert!(a.release(&one).is_some());
        assert!(a.release(&one).is_none());
    }
    #[test]
    fn paused_active_claim_does_not_expire() {
        let mut a = Admissions::default();
        let now = Instant::now();
        let o = owner(Uuid::new_v4(), "a");
        a.reserve(o.clone(), 0, true, Some(0), &[], now).unwrap();
        a.commit(&o, now).unwrap();
        assert!(a.commit(&o, now + Duration::from_secs(600)).is_ok());
    }
    #[test]
    fn last_slot_competition_is_atomic_for_one_hundred_rounds() {
        use std::sync::{Arc, Barrier, Mutex};
        for _ in 0..100 {
            let a = Arc::new(Mutex::new(Admissions::default()));
            let barrier = Arc::new(Barrier::new(2));
            let threads: Vec<_> = (0..2)
                .map(|i| {
                    let a = a.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        a.lock()
                            .unwrap()
                            .reserve(
                                owner(Uuid::new_v4(), &i.to_string()),
                                3,
                                true,
                                Some(i),
                                &[],
                                Instant::now(),
                            )
                            .is_ok()
                    })
                })
                .collect();
            assert_eq!(
                threads
                    .into_iter()
                    .map(|t| usize::from(t.join().unwrap()))
                    .sum::<usize>(),
                1
            );
        }
    }
}
