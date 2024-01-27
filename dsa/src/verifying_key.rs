//!
//! Module containing the definition of the public key container
//!

use crate::{two, Components, Signature, OID};
use core::cmp::min;
use crypto_bigint::{
    modular::{BoxedMontyForm, BoxedMontyParams},
    BoxedUint, InvMod, NonZero, Odd,
};
use digest::Digest;
use pkcs8::{
    der::{
        asn1::{BitStringRef, UintRef},
        AnyRef, Decode, Encode,
    },
    spki, AlgorithmIdentifierRef, EncodePublicKey, SubjectPublicKeyInfoRef,
};
use signature::{hazmat::PrehashVerifier, DigestVerifier, Verifier};

/// DSA public key.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
#[must_use]
pub struct VerifyingKey {
    /// common components
    components: Components,

    /// Public component y
    y: NonZero<BoxedUint>,
}

impl VerifyingKey {
    /// Construct a new public key from the common components and the public component
    pub fn from_components(
        components: Components,
        y: NonZero<BoxedUint>,
    ) -> signature::Result<Self> {
        let params = BoxedMontyParams::new_vartime(Odd::new((**components.p()).clone()).unwrap());
        let form = BoxedMontyForm::new((*y).clone(), params);

        if *y < two() || form.pow(components.q()).retrieve() != BoxedUint::one() {
            return Err(signature::Error::new());
        }

        Ok(Self { components, y })
    }

    /// DSA common components
    pub const fn components(&self) -> &Components {
        &self.components
    }

    /// DSA public component
    #[must_use]
    pub const fn y(&self) -> &NonZero<BoxedUint> {
        &self.y
    }

    /// Verify some prehashed data
    #[must_use]
    fn verify_prehashed(&self, hash: &[u8], signature: &Signature) -> Option<bool> {
        let components = self.components();
        let (p, q, g) = (components.p(), components.q(), components.g());
        let (r, s) = (signature.r(), signature.s());
        let y = self.y();

        if signature.r() >= q || signature.s() >= q {
            return Some(false);
        }

        let w = Option::from(s.inv_mod(q))?;

        let n = q.bits() / 8;
        let block_size = hash.len(); // Hash function output size

        let z_len = min(n as usize, block_size);
        let z = BoxedUint::from_be_slice(&hash[..z_len], z_len as u32 * 8).unwrap();

        let u1 = (&z * &w) % q;
        let u2 = r.mul_mod(&w, q);

        let u1_params = BoxedMontyParams::new(Odd::new(u1).unwrap());
        let u2_params = BoxedMontyParams::new(Odd::new(u2).unwrap());

        let g_form = BoxedMontyForm::new((**g).clone(), u1_params);
        let y_form = BoxedMontyForm::new((**y).clone(), u2_params);

        let v = (g_form.pow(p).retrieve() * y_form.pow(p).retrieve() % p) % q;

        Some(v == **r)
    }
}

impl Verifier<Signature> for VerifyingKey {
    fn verify(&self, msg: &[u8], signature: &Signature) -> Result<(), signature::Error> {
        self.verify_digest(sha2::Sha256::new_with_prefix(msg), signature)
    }
}

impl PrehashVerifier<Signature> for VerifyingKey {
    fn verify_prehash(
        &self,
        prehash: &[u8],
        signature: &Signature,
    ) -> Result<(), signature::Error> {
        if let Some(true) = self.verify_prehashed(prehash, signature) {
            Ok(())
        } else {
            Err(signature::Error::new())
        }
    }
}

impl<D> DigestVerifier<D, Signature> for VerifyingKey
where
    D: Digest,
{
    fn verify_digest(&self, digest: D, signature: &Signature) -> Result<(), signature::Error> {
        let hash = digest.finalize();

        let is_valid = self
            .verify_prehashed(&hash, signature)
            .ok_or_else(signature::Error::new)?;

        if !is_valid {
            return Err(signature::Error::new());
        }

        Ok(())
    }
}

impl EncodePublicKey for VerifyingKey {
    fn to_public_key_der(&self) -> spki::Result<spki::Document> {
        let parameters = self.components.to_der()?;
        let parameters = AnyRef::from_der(&parameters)?;
        let algorithm = AlgorithmIdentifierRef {
            oid: OID,
            parameters: Some(parameters),
        };

        let y_bytes = self.y.to_be_bytes();
        let y = UintRef::new(&y_bytes)?;
        let public_key = y.to_der()?;

        SubjectPublicKeyInfoRef {
            algorithm,
            subject_public_key: BitStringRef::new(0, &public_key)?,
        }
        .try_into()
    }
}

impl<'a> TryFrom<SubjectPublicKeyInfoRef<'a>> for VerifyingKey {
    type Error = spki::Error;

    fn try_from(value: SubjectPublicKeyInfoRef<'a>) -> Result<Self, Self::Error> {
        value.algorithm.assert_algorithm_oid(OID)?;

        let parameters = value.algorithm.parameters_any()?;
        let components = parameters.decode_as()?;
        let y = UintRef::from_der(
            value
                .subject_public_key
                .as_bytes()
                .ok_or(spki::Error::KeyMalformed)?,
        )?;

        Self::from_components(
            components,
            NonZero::new(
                BoxedUint::from_be_slice(y.as_bytes(), y.as_bytes().len() as u32 * 8).unwrap(),
            )
            .unwrap(),
        )
        .map_err(|_| spki::Error::KeyMalformed)
    }
}
