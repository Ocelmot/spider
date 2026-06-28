use link_set::{adaptors::peekable::Peekable, links::Link};

/// A link that already provides encryption. Implementing this is a security
/// claim.
/// 
/// Implementing this avoids double encrypting traffic through this link. If a
/// link has encryption, but is subpar in some way, avoid implementing this to
/// force traffic to be wrapped with the second encryption layer.
pub trait SecureLink: Link {
    /// Provides binding material for this encrypted link.
    fn binding(&self) -> &[u8];
}

// impl<L:SecureLink> SecureLink for Peekable<L>{
//     fn binding(&self) -> &[u8] {
        
//     }
// }