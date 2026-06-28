//! There may be links implemented across many different protocols or hardware.
//! In some cases, the data will already be encrypted by the underlying
//! mechanism. But in many cases it will not. Since Spider's communications
//! should be encrypted but it is not usually beneficial to double encrypt,
//! there needs to be a way to encrypt data through unencrypted links and attest
//! the identity of the other side of already encrypted links.
//! 
//! The Authenticated struct does this by providing different ways of wrapping
//! the underlying link depending on the role and nature of the link. It also
//! ensures the identity at the other end of the link.

use link_set::links::{Link, PinnedLink};

use crate::{LinkResult, Relation, SelfRelation, link_impls::{attested::Attested, encrypting::Encrypting, secure_link::SecureLink}};

pub(super) trait Establish: Link + Sized {
    type Inner;

    async fn listen(self_rel: SelfRelation, inner: Self::Inner) -> LinkResult<(Relation, Self)>;
    async fn connect(
        self_rel: SelfRelation,
        peer: Relation,
        inner: Self::Inner,
    ) -> LinkResult<(Relation, Self)>;
}

/// Holds the result of the handshake over the [Link]. Can be dissolved into the
/// proven [Relation] and the Box<dyn PinnedLink>.
pub struct Authenticated {
    link: Box<dyn PinnedLink>,
    rel: Relation,
}

impl Authenticated {
    /// Wraps the provided link with encryption. The handshake is performed from
    /// the listener's side. Can be passed any kind of [Link], but will
    /// double-encrypt if the underlying implementation also encrypts.
    pub async fn listen_encrypting<L: Link + 'static>(
        self_rel: SelfRelation,
        inner: L,
    ) -> LinkResult<Self> {
        let (rel, inner) = Encrypting::listen(self_rel, inner).await?;
        Ok(Self {
            link: Box::new(inner),
            rel,
        })
    }

    /// Performs a handshake from the listener's side, but does not add
    /// additional encryption. Requires a [SecureLink] instead of just a plain
    /// [Link]
    pub async fn listen_attested<L: SecureLink + 'static>(
        self_rel: SelfRelation,
        inner: L,
    ) -> LinkResult<Self> {
        let (rel, inner) = Attested::listen(self_rel, inner).await?;
        Ok(Self {
            link: Box::new(inner),
            rel,
        })
    }

    /// Wraps the provided link with encryption. The handshake is performed from
    /// the connecter's side. Can be passed any kind of [Link], but will
    /// double-encrypt if the underlying implementation also encrypts.
    pub async fn connect_encrypting<L: Link + 'static>(
        self_rel: SelfRelation,
        peer: Relation,
        inner: L,
    ) -> LinkResult<Self> {
        let (rel, inner) = Encrypting::connect(self_rel, peer, inner).await?;
        Ok(Self {
            link: Box::new(inner),
            rel,
        })
    }

    /// Performs a handshake from the connecter's side, but does not add
    /// additional encryption. Requires a [SecureLink] instead of just a plain
    /// [Link]
    pub async fn connect_attested<L: SecureLink + 'static>(
        self_rel: SelfRelation,
        peer: Relation,
        inner: L,
    ) -> LinkResult<Self> {
        let (rel, inner) = Attested::connect(self_rel, peer, inner).await?;
        Ok(Self {
            link: Box::new(inner),
            rel,
        })
    }

    /// Returns a reference to the [Relation] that was proven through the
    /// handshake
    pub fn relation(&self) -> &Relation {
        &self.rel
    }

    /// Gets the Relation and boxed link from the Authenticated.
    pub fn into_parts(self) -> (Relation, Box<dyn PinnedLink>) {
        (self.rel, self.link)
    }
}

/// Free standing version of [Authenticated::connect_encrypting]. It returns
/// impl link instead of a boxed link.
/// 
/// Wraps the provided link with encryption. The handshake is performed from the
/// connecter's side. Can be passed any kind of [Link], but will double-encrypt
/// if the underlying implementation also encrypts.
pub async fn connect_link_encrypting<L: Link + 'static>(sr: SelfRelation, rel: Relation, inner: L) -> LinkResult<(Relation, impl Link + 'static)> {
    Encrypting::connect(sr, rel, inner).await
}

/// Free standing version of [Authenticated::connect_attested]. It returns impl
/// link instead of a boxed link.
/// 
/// Performs a handshake from the connecter's side, but does not add additional
/// encryption. Requires a [SecureLink] instead of just a plain [Link]
pub async fn connect_link_attested<L: SecureLink + 'static>(sr: SelfRelation, rel: Relation, inner: L) -> LinkResult<(Relation, impl Link + 'static)> {
    Attested::connect(sr, rel, inner).await
}
