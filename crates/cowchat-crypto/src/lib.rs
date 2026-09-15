//! Cowchat v3 codecs and cryptographic checks shared by native clients and
//! future bindings. Public operations use bytes and scalars, never an ABI
//! depending on a Rust struct layout. This crate does not resolve trust or do
//! network I/O: a valid signature is not room membership or spend authority.
pub mod canonical;
pub mod certificates;
pub mod envelope;
mod fields;
pub mod keys;
pub mod request;
mod signatures;

/// Stable error discriminants for future language bindings. No input content
/// or key material is included in errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    Encoding = 1,
    Schema = 2,
    Signature = 3,
    Timestamp = 4,
    Scope = 5,
    Expiry = 6,
    Authority = 7,
    CertificateId = 8,
    Nonce = 9,
    Decrypt = 10,
    Limit = 11,
    Random = 12,
}

impl Error {
    pub fn name(self) -> &'static str {
        match self {
            Self::Encoding => "encoding",
            Self::Schema => "schema",
            Self::Signature => "signature",
            Self::Timestamp => "timestamp",
            Self::Scope => "scope",
            Self::Expiry => "expiry",
            Self::Authority => "authority",
            Self::CertificateId => "cert_id",
            Self::Nonce => "nonce",
            Self::Decrypt => "decrypt",
            Self::Limit => "limit",
            Self::Random => "random",
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
impl std::error::Error for Error {}

pub(crate) type Result<T> = std::result::Result<T, Error>;
