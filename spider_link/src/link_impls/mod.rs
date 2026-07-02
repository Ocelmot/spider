mod secure_link;
mod encrypting;
mod attested;
pub mod authenticated;


// Transport features
#[cfg(feature = "transport_iroh")]
pub mod iroh_link;

// mod veilid_link;
// pub use veilid_link::{VeilidLink, VeilidHub, VeilidConnector};

/// Result type for [LinkImplError], the type of error returned from implementations of the [Link] trait
pub type LinkImplResult<T = ()> = Result<T, LinkImplError>;

/// The error type for implementations of the [Link] trait
#[derive(Debug)]
pub enum LinkImplError {
	/// The link has encountered a deserialization error
	Deserialize,
	/// The link's receiver has already been taken from the link itself
    ReceiverTaken,
	/// The link has closed
    Closed,
}

impl std::error::Error for LinkImplError {}
impl std::fmt::Display for LinkImplError{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			LinkImplError::Deserialize => write!(f, "Failed to deserialize"),
			LinkImplError::ReceiverTaken => write!(f, "Receiver already taken"),
			LinkImplError::Closed => write!(f, "Link has closed"),
		}
	}
}
