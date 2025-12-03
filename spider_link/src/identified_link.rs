//! Contains the IdentifiedLink trait

use link_set::links::PinnedLink;

use crate::Relation;

/// An IdentifiedLink is a [PinnedLink] that also identifies the [Relation] of the other side of the link
pub trait IdentifiedLink: PinnedLink {
    /// Returns the [Relation] of the other side of the link
    fn other_relation(&self) -> &Relation;
}

