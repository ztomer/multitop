//! Sealing the vault key to this machine's TPM.
//!
//! # What this protects against, and what it does not
//!
//! It is tempting to read this as the Linux answer to the Secure Enclave, and
//! it is not. The macOS path is secure because the enclave key is created with
//! `kSecAccessControlBiometryCurrentSet`: the *hardware* refuses to use the key
//! unless Touch ID succeeds, and no code in this process ever sees key
//! material. A TPM has no such tie to `fprintd`. `fprintd` is a userspace
//! daemon that answers yes or no over D-Bus; the TPM cannot ask it anything and
//! cannot be told to wait for it.
//!
//! So, precisely:
//!
//! * **Protected:** a vault file copied to another machine. The sealed blob is
//!   bound to this TPM's storage seed and unseals nowhere else, so an attacker
//!   who takes the file gets only the Argon2id wrapper and has to guess the
//!   master password -- which is the property the file permissions and the
//!   lockout limiter already exist to defend.
//! * **Not protected:** a local attacker already running as this user. They can
//!   unseal exactly as this code does, without presenting a finger. The
//!   fingerprint is a convenience gate, not a cryptographic one.
//!
//! Nothing here or in the UI may describe this as biometric protection. It is
//! machine binding, and the fingerprint prompt is what makes it convenient.
//!
//! # Why it does not bind to PCRs
//!
//! Sealing against PCR values would additionally require the boot state to
//! match, which sounds strictly better and is a footgun: a kernel update, a
//! firmware update or a bootloader change silently makes the vault unopenable
//! by fingerprint, with no way back except the master password and a re-seal.
//! The macOS side already has a repair path for exactly that class of surprise
//! (`should_rebind_biometric`) and it exists because the invalidation happened
//! in practice. Machine binding without boot binding is the property worth
//! having here; if boot binding is wanted later it needs the repair path first.

#[cfg(target_os = "linux")]
pub use imp::{is_available, seal, unseal};
#[cfg(not(target_os = "linux"))]
pub use stub::{is_available, seal, unseal};

/// The TPM's own limit on a sealed payload is well above a 32-byte key; this
/// bounds what will be *read back*, so a corrupt or hostile header cannot make
/// the unseal path allocate freely.
pub const MAX_SEALED_BYTES: usize = 4096;

#[cfg(not(target_os = "linux"))]
mod stub {
    use crate::crypto::VaultKey;
    use crate::VaultError;

    /// No TPM outside Linux. macOS has the Secure Enclave and goes through
    /// `secure_enclave`; everything else has neither.
    #[must_use]
    pub const fn is_available() -> bool {
        false
    }

    /// # Errors
    /// Always `PlatformNotSupported`.
    pub fn seal(_key: &VaultKey) -> Result<Vec<u8>, VaultError> {
        Err(VaultError::PlatformNotSupported(
            "TPM2 sealing is Linux only".into(),
        ))
    }

    /// # Errors
    /// Always `PlatformNotSupported`.
    pub fn unseal(_blob: &[u8]) -> Result<VaultKey, VaultError> {
        Err(VaultError::PlatformNotSupported(
            "TPM2 sealing is Linux only".into(),
        ))
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use tss_esapi::attributes::ObjectAttributesBuilder;
    use tss_esapi::handles::KeyHandle;
    use tss_esapi::interface_types::algorithm::{HashingAlgorithm, PublicAlgorithm};
    use tss_esapi::interface_types::key_bits::RsaKeyBits;
    use tss_esapi::interface_types::resource_handles::Hierarchy;
    use tss_esapi::structures::{
        CreatePrimaryKeyResult, Digest, KeyedHashScheme, Private, Public, PublicBuilder,
        PublicKeyRsa, PublicKeyedHashParameters, PublicRsaParametersBuilder, RsaExponent,
        SensitiveData, SymmetricDefinitionObject,
    };
    use tss_esapi::{Context, TctiNameConf};

    use crate::crypto::VaultKey;
    use crate::VaultError;

    use super::MAX_SEALED_BYTES;

    /// Where to find the TPM.
    ///
    /// From the environment, which is what `tpm2-tools` and every other TPM
    /// consumer read, so a software TPM under test and a real one in production
    /// are selected the same way rather than by a flag this crate invents.
    /// Falling back to the device is what a normal Linux box wants.
    fn tcti() -> Result<TctiNameConf, VaultError> {
        TctiNameConf::from_environment_variable().or_else(|_| {
            "device:/dev/tpmrm0"
                .parse()
                .map_err(|e| VaultError::Tpm2Error(format!("no usable TPM interface: {e}")))
        })
    }

    fn context() -> Result<Context, VaultError> {
        Context::new(tcti()?).map_err(|e| VaultError::Tpm2Error(format!("TPM unavailable: {e}")))
    }

    /// Whether this machine has a TPM this code can talk to.
    ///
    /// Opening a context is the only honest test: a `/dev/tpmrm0` that exists
    /// but cannot be opened, a resource manager that is not running, and a
    /// permission problem all look identical from a `stat`.
    #[must_use]
    pub fn is_available() -> bool {
        context().is_ok()
    }

    /// The parent under which the sealed object lives.
    ///
    /// Derived from the owner hierarchy's seed and a fixed template, so it is
    /// the same key on every run without anything being stored: the TPM
    /// regenerates it deterministically. That is what makes the sealed blob in
    /// the vault header sufficient on its own -- there is no second piece of
    /// state to keep in step, and no handle to leak if the process dies between
    /// creating and persisting.
    fn primary(context: &mut Context) -> Result<CreatePrimaryKeyResult, VaultError> {
        let attributes = ObjectAttributesBuilder::new()
            .with_fixed_tpm(true)
            .with_fixed_parent(true)
            .with_sensitive_data_origin(true)
            .with_user_with_auth(true)
            .with_decrypt(true)
            .with_restricted(true)
            .build()
            .map_err(|e| VaultError::Tpm2Error(format!("primary attributes: {e}")))?;

        let public = PublicBuilder::new()
            .with_public_algorithm(PublicAlgorithm::Rsa)
            .with_name_hashing_algorithm(HashingAlgorithm::Sha256)
            .with_object_attributes(attributes)
            .with_rsa_parameters(
                PublicRsaParametersBuilder::new_restricted_decryption_key(
                    SymmetricDefinitionObject::AES_128_CFB,
                    RsaKeyBits::Rsa2048,
                    RsaExponent::default(),
                )
                .build()
                .map_err(|e| VaultError::Tpm2Error(format!("primary parameters: {e}")))?,
            )
            .with_rsa_unique_identifier(PublicKeyRsa::default())
            .build()
            .map_err(|e| VaultError::Tpm2Error(format!("primary template: {e}")))?;

        context
            .execute_with_nullauth_session(|ctx| {
                ctx.create_primary(Hierarchy::Owner, public, None, None, None, None)
            })
            .map_err(|e| VaultError::Tpm2Error(format!("creating the primary key: {e}")))
    }

    /// The template for the sealed object itself: a keyed-hash object with no
    /// scheme, which is how a TPM stores arbitrary bytes.
    fn sealed_template() -> Result<Public, VaultError> {
        let attributes = ObjectAttributesBuilder::new()
            .with_fixed_tpm(true)
            .with_fixed_parent(true)
            .with_user_with_auth(true)
            .build()
            .map_err(|e| VaultError::Tpm2Error(format!("sealed attributes: {e}")))?;

        PublicBuilder::new()
            .with_public_algorithm(PublicAlgorithm::KeyedHash)
            .with_name_hashing_algorithm(HashingAlgorithm::Sha256)
            .with_object_attributes(attributes)
            .with_keyed_hash_parameters(PublicKeyedHashParameters::new(KeyedHashScheme::Null))
            .with_keyed_hash_unique_identifier(Digest::default())
            .build()
            .map_err(|e| VaultError::Tpm2Error(format!("sealed template: {e}")))
    }

    /// Seal the vault key to this TPM, returning what the header should store.
    ///
    /// # Errors
    /// `Tpm2Error` if there is no reachable TPM or it refuses the operation.
    pub fn seal(key: &VaultKey) -> Result<Vec<u8>, VaultError> {
        let mut context = context()?;
        let parent = primary(&mut context)?;

        let data = SensitiveData::try_from(key.as_bytes().to_vec())
            .map_err(|e| VaultError::Tpm2Error(format!("the key is not sealable: {e}")))?;

        // Built before the session, not inside it: the closure has to return
        // the TPM's own error type, so a `?` on ours cannot live in there.
        let template = sealed_template()?;
        let created = context
            .execute_with_nullauth_session(|ctx| {
                ctx.create(parent.key_handle, template, None, Some(data), None, None)
            })
            .map_err(|e| VaultError::Tpm2Error(format!("sealing: {e}")))?;

        encode(&created.out_public, &created.out_private)
    }

    /// Unseal a blob this machine's TPM produced.
    ///
    /// # Errors
    /// `Tpm2Error` if there is no reachable TPM, the blob is not this TPM's, or
    /// it does not decode.
    pub fn unseal(blob: &[u8]) -> Result<VaultKey, VaultError> {
        let (public, private) = decode(blob)?;
        let mut context = context()?;
        let parent = primary(&mut context)?;

        let sealed: KeyHandle = context
            .execute_with_nullauth_session(|ctx| ctx.load(parent.key_handle, private, public))
            .map_err(|e| VaultError::Tpm2Error(format!("loading the sealed object: {e}")))?;

        let data = context
            .execute_with_nullauth_session(|ctx| ctx.unseal(sealed.into()))
            .map_err(|e| VaultError::Tpm2Error(format!("unsealing: {e}")))?;

        let bytes: [u8; 32] = data
            .value()
            .try_into()
            .map_err(|_| VaultError::Tpm2Error("the sealed data is not a vault key".into()))?;
        Ok(VaultKey::from_bytes(bytes))
    }

    /// `len(public) || public || len(private) || private`, both `u16`.
    ///
    /// Written out here rather than with a serialisation crate because this
    /// goes in a versioned on-disk header: a derive that reorders or renames a
    /// field silently changes the bytes, and the failure would be a vault that
    /// stops opening after a dependency bump.
    fn encode(public: &Public, private: &Private) -> Result<Vec<u8>, VaultError> {
        let public = tss_esapi::traits::Marshall::marshall(public)
            .map_err(|e| VaultError::Tpm2Error(format!("encoding the public part: {e}")))?;
        let private: &[u8] = private.as_ref();

        let (Ok(pub_len), Ok(priv_len)) =
            (u16::try_from(public.len()), u16::try_from(private.len()))
        else {
            return Err(VaultError::Tpm2Error(
                "the TPM produced a blob too large for the header".into(),
            ));
        };

        let mut out = Vec::with_capacity(public.len() + private.len() + 4);
        out.extend_from_slice(&pub_len.to_le_bytes());
        out.extend_from_slice(&public);
        out.extend_from_slice(&priv_len.to_le_bytes());
        out.extend_from_slice(private);
        Ok(out)
    }

    fn decode(blob: &[u8]) -> Result<(Public, Private), VaultError> {
        let bad = |what: &str| VaultError::Tpm2Error(format!("the sealed blob is {what}"));

        if blob.len() > MAX_SEALED_BYTES {
            return Err(bad("larger than a sealed key can be"));
        }
        let pub_len = read_len(blob, 0).ok_or_else(|| bad("truncated before the public part"))?;
        let pub_end = 2 + pub_len;
        let public = blob.get(2..pub_end).ok_or_else(|| bad("truncated"))?;
        let priv_len =
            read_len(blob, pub_end).ok_or_else(|| bad("truncated before the private part"))?;
        let private = blob
            .get(pub_end + 2..pub_end + 2 + priv_len)
            .ok_or_else(|| bad("truncated"))?;

        let public = <Public as tss_esapi::traits::UnMarshall>::unmarshall(public)
            .map_err(|e| VaultError::Tpm2Error(format!("the public part does not decode: {e}")))?;
        let private = Private::try_from(private.to_vec())
            .map_err(|e| VaultError::Tpm2Error(format!("the private part does not decode: {e}")))?;
        Ok((public, private))
    }

    fn read_len(blob: &[u8], at: usize) -> Option<usize> {
        let bytes: [u8; 2] = blob.get(at..at + 2)?.try_into().ok()?;
        Some(u16::from_le_bytes(bytes) as usize)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::imp::{seal, unseal};
    use super::MAX_SEALED_BYTES;
    use crate::crypto::VaultKey;

    /// Everything here that does not need a TPM: the framing around the blob.
    /// A header is attacker-reachable -- it is a file on disk -- so a malformed
    /// one has to be refused rather than trusted to describe its own length.
    #[test]
    fn a_blob_that_lies_about_its_length_is_refused() {
        for bad in [
            vec![],                             // nothing at all
            vec![0x10],                         // half a length
            vec![0xff, 0xff],                   // a length with no body
            vec![0x04, 0x00, 1, 2],             // public shorter than promised
            vec![0x02, 0x00, 1, 2, 0xff, 0x7f], // private length past the end
        ] {
            // `err()` rather than `expect_err`: the `Ok` side is a `VaultKey`,
            // which has no `Debug` on purpose, so it cannot be printed into a
            // panic message. The type refusing to be formatted is the point.
            let Some(err) = unseal(&bad).err() else {
                panic!("a malformed blob unsealed: {bad:?}");
            };
            assert!(
                matches!(err, crate::VaultError::Tpm2Error(_)),
                "wrong error for {bad:?}: {err}"
            );
        }
    }

    #[test]
    fn a_blob_larger_than_a_sealed_key_can_be_is_refused_before_it_is_parsed() {
        // The bound exists so a corrupt header cannot make this allocate
        // freely; it must be checked before any length inside is believed.
        let huge = vec![0u8; MAX_SEALED_BYTES + 1];
        assert!(unseal(&huge).is_err());
    }

    /// The round trip, against whatever TPM this machine has.
    ///
    /// `#[ignore]`d for the same reason the SSH tests are: it needs hardware CI
    /// does not have, and access to `/dev/tpmrm0`, which is `tss`-group only.
    /// A test that silently passes when it did nothing is worse than one that
    /// is visibly not run -- that is the mistake the old fuzz gate made.
    ///
    /// Run it on a machine with a TPM:
    ///     sudo -E cargo test -p multitop-vault --lib tpm2 -- --ignored --nocapture
    #[test]
    #[ignore = "needs a real TPM and access to /dev/tpmrm0"]
    fn a_sealed_key_comes_back_from_this_machines_tpm() {
        assert!(
            super::is_available(),
            "no reachable TPM; this test cannot say anything"
        );
        let key = VaultKey::new();
        let want = key.as_bytes().to_vec();

        let blob = seal(&key).expect("sealing");
        assert!(blob.len() <= MAX_SEALED_BYTES, "{} bytes", blob.len());

        // Compared as bytes: `VaultKey` deliberately has no `Debug`, so an
        // assertion that could print one would not compile -- which is the type
        // doing its job.
        let back = unseal(&blob).expect("unsealing");
        assert!(
            back.as_bytes() == want.as_slice(),
            "the key that came back is not the key that went in"
        );

        // A blob whose ciphertext has been edited must fail the TPM's own
        // integrity check rather than returning something.
        let mut edited = blob;
        let last = edited.len() - 1;
        edited[last] ^= 0xff;
        assert!(unseal(&edited).is_err(), "an edited blob unsealed");
    }
}
