use rand::{OsRng, Rng};
use sha2::{self,Digest};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::edwards::{EdwardsPoint, CompressedEdwardsY};
use curve25519_dalek::constants;
use curve25519_dalek::traits::Identity;
use std::ops::Add;
use std::ops::AddAssign;
use std::fmt;
use std::error;
use super::{KeyPair,PublicKey,Signature};
use std::iter::Sum;
use std::borrow::Borrow;

#[derive(PartialEq, Eq, Debug)]
pub struct RandomSecret(Scalar);

impl RandomSecret {
    pub const SIZE: usize = 32;
}

impl From<[u8; RandomSecret::SIZE]> for RandomSecret {
    fn from(bytes: [u8; RandomSecret::SIZE]) -> Self {
        return RandomSecret(Scalar::from_bytes_mod_order(bytes));
    }
}

#[derive(PartialEq, Eq, Debug)]
pub struct Commitment(EdwardsPoint);
implement_simple_add_sum_traits!(Commitment, EdwardsPoint::identity());

impl Commitment {
    pub const SIZE: usize = 32;

    #[inline]
    pub fn to_bytes(&self) -> [u8; Commitment::SIZE] {
        self.0.compress().to_bytes()
    }

    pub fn from_bytes(bytes: [u8; Commitment::SIZE]) -> Option<Self> {
        let compressed = CompressedEdwardsY(bytes);
        return match compressed.decompress() {
            None => None,
            Some(e) => Some(Commitment(e)),
        };
    }
}

impl From<[u8; Commitment::SIZE]> for Commitment {
    fn from(bytes: [u8; Commitment::SIZE]) -> Self {
        return Commitment::from_bytes(bytes).unwrap();
    }
}

#[derive(Debug, Clone)]
pub struct InvalidScalarError;

impl fmt::Display for InvalidScalarError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        return write!(f, "Generated scalar was invalid (0 or 1).");
    }
}

impl error::Error for InvalidScalarError {
    fn description(&self) -> &str {
        "Generated scalar was invalid (0 or 1)."
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

#[derive(PartialEq, Eq, Debug)]
pub struct CommitmentPair {
    random_secret: RandomSecret,
    commitment: Commitment
}

impl CommitmentPair {
    pub fn generate() -> Result<CommitmentPair, InvalidScalarError> {
        // Create random 32 bytes.
        let mut cspring: OsRng = OsRng::new().unwrap();
        let mut randomness: [u8; RandomSecret::SIZE] = [0u8; RandomSecret::SIZE];
        cspring.fill_bytes(&mut randomness);

        // Decompress the 32 byte cryptographically secure random data to 64 byte.
        let mut h: sha2::Sha512 = sha2::Sha512::default();

        h.input(&randomness);
        let scalar = Scalar::from_hash::<sha2::Sha512>(h);
        if scalar == Scalar::zero() || scalar == Scalar::one() {
            return Err(InvalidScalarError);
        }

        // Compute the point [scalar]B.
        let commitment: EdwardsPoint = &scalar * &constants::ED25519_BASEPOINT_TABLE;

        let rs = RandomSecret(scalar);
        let ct = Commitment(commitment);
        return Ok(CommitmentPair { random_secret: rs, commitment: ct });
    }
}

#[derive(PartialEq, Eq, Debug)]
pub struct PartialSignature (Scalar);
implement_simple_add_sum_traits!(PartialSignature, Scalar::zero());

impl PartialSignature {
    pub const SIZE: usize = 32;

    #[inline]
    pub fn as_bytes<'a>(&'a self) -> &'a [u8; PartialSignature::SIZE] { self.0.as_bytes() }
}

impl From<[u8; PartialSignature::SIZE]> for PartialSignature {
    fn from(bytes: [u8; PartialSignature::SIZE]) -> Self {
        return PartialSignature(Scalar::from_bytes_mod_order(bytes));
    }
}

pub fn partial_signature_create(key_pair: &KeyPair, public_keys: &mut Vec<PublicKey>, secret: &RandomSecret, commitments: &Vec<Commitment>, data: &[u8]) -> (PartialSignature, PublicKey, Commitment) {
    if public_keys.len() != commitments.len() {
        panic!("Number of public keys and commitments must be the same.");
    }
    if public_keys.len() == 0 {
        panic!("Number of public keys and commitments must be greater than 0.");
    }

    // Sort public keys.
    public_keys.sort();

    // Hash public keys.
    let public_keys_hash = hash_public_keys(public_keys);
    // And delinearize them.
    let delinearized_pk_sum: EdwardsPoint = public_keys.iter().map(|public_key| { delinearize_public_key(public_key, &public_keys_hash) }).sum();
    let delinearized_private_key: Scalar = delinearize_private_key(key_pair, &public_keys_hash);

    // Aggregate commitments.
    let aggregated_commitment: Commitment = commitments.iter().sum();

    // Compute H(commitment || public key || message).
    let mut h: sha2::Sha512 = sha2::Sha512::default();

    h.input(aggregated_commitment.0.compress().as_bytes());
    h.input(delinearized_pk_sum.compress().as_bytes());
    h.input(data);
    let s = Scalar::from_hash::<sha2::Sha512>(h);
    let partial_signature: Scalar = s * delinearized_private_key + secret.0;
    let mut public_key_bytes : [u8; PublicKey::SIZE] = [0u8; PublicKey::SIZE];
    public_key_bytes.copy_from_slice(delinearized_pk_sum.compress().as_bytes());
    return (PartialSignature(partial_signature), PublicKey::from(public_key_bytes), aggregated_commitment);
}

fn hash_public_keys(public_keys: &Vec<PublicKey>) -> [u8; 64] {
    let mut aggregated_public_key: Option<PublicKey> = None;
    // 1. Compute hash over public keys public_keys_hash = C = H(P_1 || ... || P_n).
    let mut h: sha2::Sha512 = sha2::Sha512::default();
    let mut public_keys_hash: [u8; 64] = [0u8; 64];
    for public_key in public_keys {
        h.input(public_key.as_bytes());
    }
    public_keys_hash.copy_from_slice(h.result().as_slice());
    return public_keys_hash;
}

impl PublicKey {
    fn to_edwards_point(&self) -> Option<EdwardsPoint> {
        let mut bits: [u8; PublicKey::SIZE] = [0u8; PublicKey::SIZE];
        bits.copy_from_slice(&self.as_bytes()[..PublicKey::SIZE]);

        let compressed = CompressedEdwardsY(bits);
        return compressed.decompress();
    }
}

fn delinearize_public_key(public_key: &PublicKey, public_keys_hash: &[u8; 64]) -> EdwardsPoint {
    // Compute H(C||P).
    let mut h: sha2::Sha512 = sha2::Sha512::default();

    h.input(public_keys_hash);
    h.input(public_key.as_bytes());
    let s = Scalar::from_hash::<sha2::Sha512>(h);

    // Should always work, since we come from a valid public key.
    let p = public_key.to_edwards_point().unwrap();
    // Compute H(C||P)*P.
    return s * p;
}

trait ToScalar {
    fn to_scalar(&self) -> Scalar;
}

impl ToScalar for ::ed25519_dalek::ExpandedSecretKey {
    fn to_scalar(&self) -> Scalar {
        let mut bytes: [u8; 32] = [0u8; 32];
        bytes.copy_from_slice(&self.to_bytes()[..32]);
        return Scalar::from_bytes_mod_order(bytes);
    }
}

fn delinearize_private_key(key_pair: &KeyPair, public_keys_hash: &[u8; 64]) -> Scalar {
    // Compute H(C||P).
    let mut h: sha2::Sha512 = sha2::Sha512::default();

    h.input(public_keys_hash);
    h.input(key_pair.public().as_bytes());
    let s = Scalar::from_hash::<sha2::Sha512>(h);

    // Expand the private key.
    let expanded_private_key = key_pair.private().as_dalek().expand::<sha2::Sha512>();
    let sk = expanded_private_key.to_scalar();

    // Compute H(C||P)*sk
    return s * sk;
}

impl Signature {
    pub fn from_multisig(aggregated_signature: &PartialSignature, aggregated_commitment: &Commitment) -> Signature {
        let mut signature: [u8; Signature::SIZE] = [0u8; Signature::SIZE];
        signature[..Commitment::SIZE].copy_from_slice(&aggregated_commitment.to_bytes());
        signature[Commitment::SIZE..].copy_from_slice(aggregated_signature.as_bytes());
        return Signature::from(&signature);
    }
}
