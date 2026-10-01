//! Bounded, in-memory single-use grants. Restart deliberately expires grants.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub version: u16,
    pub hub_id: Uuid,
    pub room_name: String,
    pub certificate: String,
    pub invitation_id: Uuid,
    pub secret: String,
    pub expires_unix_seconds: u64,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub invitation_id: Uuid,
    pub request_id: Uuid,
    pub name: String,
    pub token_sha256: String,
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || self.token_sha256.len() != 64
            || !self
                .token_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Completion {
    pub hub_id: Uuid,
    pub device_id: Uuid,
    pub revision: u64,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("pairing_unauthorized_or_expired")]
    Unauthorized,
    #[error("pairing_invalid_argument")]
    Invalid,
    #[error("pairing_already_used")]
    Used,
    #[error("pairing_quota_exceeded")]
    Quota,
}
struct Grant<I> {
    issuer: I,
    digest: String,
    deadline: Instant,
    completion: Option<(Request, Completion)>,
}
pub enum Checked<I> {
    New(I),
    Repeated(Completion),
}
pub struct Book<I> {
    grants: BTreeMap<Uuid, Grant<I>>,
}
impl<I: Clone> Default for Book<I> {
    fn default() -> Self {
        Self {
            grants: BTreeMap::new(),
        }
    }
}
impl<I: Clone> Book<I> {
    pub fn open(
        &mut self,
        issuer: I,
        ttl_seconds: u32,
        now: Instant,
    ) -> Result<(Uuid, String), Error> {
        if !(1..=300).contains(&ttl_seconds) {
            return Err(Error::Invalid);
        }
        self.grants.retain(|_, grant| now < grant.deadline);
        if self.grants.len() >= 16 {
            return Err(Error::Quota);
        }
        let id = Uuid::new_v4();
        let secret = crate::secret();
        self.grants.insert(
            id,
            Grant {
                issuer,
                digest: crate::digest(&secret),
                deadline: now + Duration::from_secs(u64::from(ttl_seconds)),
                completion: None,
            },
        );
        Ok((id, secret))
    }
    pub fn check(
        &self,
        secret: &str,
        request: &Request,
        now: Instant,
    ) -> Result<Checked<I>, Error> {
        let grant = self
            .grants
            .get(&request.invitation_id)
            .ok_or(Error::Unauthorized)?;
        if secret.len() != 64
            || now >= grant.deadline
            || !bool::from(
                grant
                    .digest
                    .as_bytes()
                    .ct_eq(crate::digest(secret).as_bytes()),
            )
        {
            return Err(Error::Unauthorized);
        }
        request.validate()?;
        match &grant.completion {
            Some((previous, result)) if previous == request => {
                Ok(Checked::Repeated(result.clone()))
            }
            Some(_) => Err(Error::Used),
            None => Ok(Checked::New(grant.issuer.clone())),
        }
    }
    // Caller holds its transaction lock from check through durable registration.
    pub fn finish(&mut self, request: Request, completion: Completion) {
        if let Some(grant) = self.grants.get_mut(&request.invitation_id) {
            grant.completion = Some((request, completion));
        }
    }
    pub fn cancel(&mut self, id: Uuid) {
        self.grants.remove(&id);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expiration_cancellation_single_use_and_exact_retry() {
        let now = Instant::now();
        let mut book = Book::default();
        let (id, secret) = book.open(7, 30, now).unwrap();
        let mut request = Request {
            invitation_id: id,
            request_id: Uuid::new_v4(),
            name: "Sender".into(),
            token_sha256: crate::digest(&crate::secret()),
        };
        assert!(matches!(
            book.check(&secret, &request, now),
            Ok(Checked::New(7))
        ));
        assert!(matches!(
            book.check(&crate::secret(), &request, now),
            Err(Error::Unauthorized)
        ));
        assert!(matches!(
            book.check(&secret, &request, now + Duration::from_secs(30)),
            Err(Error::Unauthorized)
        ));
        let done = Completion {
            hub_id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            revision: 2,
        };
        book.finish(request.clone(), done.clone());
        assert!(
            matches!(book.check(&secret,&request,now), Ok(Checked::Repeated(c)) if c.device_id==done.device_id)
        );
        request.request_id = Uuid::new_v4();
        assert!(matches!(
            book.check(&secret, &request, now),
            Err(Error::Used)
        ));
        book.cancel(id);
        assert!(matches!(
            book.check(&secret, &request, now),
            Err(Error::Unauthorized)
        ));
    }
    #[test]
    fn quota_reclaims_expired_grants() {
        let now = Instant::now();
        let mut book = Book::default();
        for _ in 0..16 {
            book.open(1, 1, now).unwrap();
        }
        assert_eq!(book.open(1, 1, now).unwrap_err(), Error::Quota);
        assert!(book.open(1, 1, now + Duration::from_secs(1)).is_ok());
    }
}
