//! Exact certificate pinning with normal TLS CertificateVerify validation.
//! DNS names are routing hints: a saved pin authenticates a moved Hub.
use crate::Result;
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::sync::Arc;
#[derive(Debug)]
struct Pinned {
    certificate: CertificateDer<'static>,
    provider: rustls::crypto::CryptoProvider,
}
impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if end_entity != &self.certificate || !intermediates.is_empty() {
            return Err(rustls::Error::General(
                "Hub identity changed; explicit pairing required".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
pub fn tls(pem: &str) -> Result<rustls::ClientConfig> {
    let mut certificates = rustls_pemfile::certs(&mut std::io::Cursor::new(pem))
        .collect::<std::io::Result<Vec<_>>>()?;
    if certificates.len() != 1 {
        return Err("exactly one pinned Hub certificate required".into());
    }
    let provider = rustls::crypto::ring::default_provider();
    let verifier = Pinned {
        certificate: certificates.remove(0),
        provider: provider.clone(),
    };
    Ok(
        rustls::ClientConfig::builder_with_provider(Arc::new(provider))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth(),
    )
}
