//! Offline regression for the CA key distrust incident tracked as CVE-2026-111943.
//!
//! Run against both lockfiles:
//! cargo test -p moltis-client --test ca_distrust
//! cargo test --manifest-path apps/desktop/Cargo.toml --test ca_distrust

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::BTreeSet, sync::Arc};

use {
    rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose},
    rustls::{
        RootCertStore,
        client::{WebPkiServerVerifier, danger::ServerCertVerifier},
        pki_types::{ServerName, SubjectPublicKeyInfoDer, UnixTime, pem::PemObject},
    },
    sha2::{Digest, Sha256},
};

// SHA-256 of complete DER SubjectPublicKeyInfo, from Go's upstream fix:
// https://github.com/golang/go/commit/7f86f957633ee31a1c17a1eb8452ca7a3d9bc141
const DISTRUSTED_SPKI_HASHES: [&str; 8] = [
    "ebed6475c2c2d80fcb199f3fe8dd6e6b3ce60b77eb15d4e42c3dec0c6f5b5105",
    "49dd353dc2b919a3770645117d0e60c16115e7d1bc259eefe734b00ea434fe5f",
    "e0254c7fcdc265e2f0f8a428dabb75ab4ae039b4872d62f5ee57f0877d5a8a41",
    "5c0660a2b164910d8ff589c18e14552e9e0ec237dcefe2cb736d41a4811f81a0",
    "43bffa19d6ef651451f6841e019931e96f6f2eaeff28ab4b6f1ce4576de2717b",
    "63ecefee431d49523bc1136c4e72d0206a477086caf606cd5cf025686c109cf8",
    "ec4ffa1ad0ac88081314b7aaffef6a91f97cd57291bb5ba686c5415f5eb4f6b6",
    "92c063d67e8f14db71d288e11266ffdaf69cbf115f7bab14f3bc2f68d25e0e30",
];

#[test]
fn distrusted_ca_keys_rejected_even_when_resigned_as_leaves() {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    let verifier = WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap();
    let server_name = ServerName::try_from("localhost").unwrap();
    let now = UnixTime::now();

    // Positive control: the same CA, validity, hostname and signature scheme
    // must work for an ordinary key, including with a custom trust store.
    let leaf_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
    let good_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap();
    let good_leaf = leaf_params
        .clone()
        .signed_by(&good_key, &ca, &ca_key)
        .unwrap();
    verifier
        .verify_server_cert(good_leaf.der(), &[], &server_name, &[], now)
        .unwrap();

    let mut seen = BTreeSet::new();
    for key in
        SubjectPublicKeyInfoDer::pem_slice_iter(include_bytes!("fixtures/distrusted_ca_keys.pem"))
    {
        let key = key.unwrap();
        let hash = format!("{:x}", Sha256::digest(key.as_ref()));
        assert!(DISTRUSTED_SPKI_HASHES.contains(&hash.as_str()));
        assert!(seen.insert(hash.clone()), "duplicate fixture key: {hash}");
        let public_key = rcgen::SubjectPublicKeyInfo::from_der(key.as_ref()).unwrap();

        // Reissue the public key under our fresh, explicitly trusted CA. The
        // chain is otherwise valid: rejection must not depend on the original
        // issuer, certificate fingerprint, expiry or a missing trust anchor.
        // No private key belonging to a distrusted CA is needed or included.
        let leaf = leaf_params
            .clone()
            .signed_by(&public_key, &ca, &ca_key)
            .unwrap();
        let error = verifier
            .verify_server_cert(leaf.der(), &[], &server_name, &[], now)
            .expect_err("a distrusted public key must be rejected under any issuer");
        assert!(matches!(error, rustls::Error::InvalidCertificate(_)));
    }
    assert_eq!(seen.len(), DISTRUSTED_SPKI_HASHES.len());
}
