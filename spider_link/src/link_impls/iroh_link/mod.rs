//! Implementation of Link and SecureLink for the Iroh protocol

use iroh::endpoint::{Connection, SendDatagramError};
use link_set::links::{Link, LinkReader};
use tracing::error;

use crate::{
    error::{ErrorKind, ProblemWrap},
    link_impls::secure_link::SecureLink,
    LinkResult,
};

/// Implementation of [Link] over the Iroh system
pub struct IrohLink {
    conn: Connection,
    binding: Vec<u8>,
    taken: bool,
    closed: bool,
}

impl IrohLink {
    /// Create a new IrohLink from an [iroh::endpoint::Connection]
    pub fn new(conn: Connection) -> Self {
        let mut binding = [0u8; 32];
        conn.export_keying_material(&mut binding, b"spider_iroh_link", b"").expect("requested binding length is within bounds");
        let binding = binding.to_vec();
        Self {
            conn,
            binding,
            taken: false,
            closed: false,
        }
    }
}

/// The [LinkReader] when taken from [IrohLink]
pub struct IrohLinkReader(Connection);

impl LinkReader for IrohLinkReader {
    async fn read(&mut self) -> Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static> {
        match self.0.read_datagram().await {
            Ok(data) => LinkResult::Ok(data.to_vec()),
            Err(e) => Err(e).wrap_msg("Read failed")?,
        }
    }
}

impl Link for IrohLink {
    async fn send(
        &mut self,
        msg: Vec<u8>,
    ) -> Result<(), impl std::error::Error + Send + Sync + 'static> {
        let result = self.conn.send_datagram(msg.into());

        if matches!(result, Err(SendDatagramError::TooLarge)) {
            error!("Iroh was passed too large data!. max_size() not respected");
        }
        if matches!(result, Err(SendDatagramError::Disabled) | Err(SendDatagramError::UnsupportedByPeer)) {
            self.closed = true;
            result.wrap_problem(ErrorKind::Closed)?;
        }
        
        LinkResult::Ok(())
    }

    async fn recv(&mut self) -> Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static> {
        if self.taken {
            Err(ErrorKind::Taken)?
        }
        match self.conn.read_datagram().await {
            Ok(data) => LinkResult::Ok(data.to_vec()),
            Err(e) => {
                self.closed = true;
                Err(e).wrap_msg("Recv failed")?
            }
        }
    }

    async fn close(&mut self) -> Result<(), impl std::error::Error + Send + Sync + 'static> {
        self.closed = true;
        self.conn.close(0u8.into(), b"Peer Closed");
        LinkResult::Ok(())
    }

    fn take_reader(
        &mut self,
    ) -> Result<
        impl link_set::links::LinkReader + 'static,
        impl std::error::Error + Send + Sync + 'static,
    > {
        if self.taken {
            Err(ErrorKind::Taken)?
        }
        self.taken = true;
        let lr = IrohLinkReader(self.conn.clone());
        LinkResult::Ok(lr)
    }

    fn max_size(&self) -> u32 {
        1024 // Low estimate for link capacity
    }

    fn is_closed(&mut self) -> bool {
        self.closed || self.conn.close_reason().is_some()
    }
}

impl SecureLink for IrohLink {
    fn binding(&self) -> &[u8] {
        &self.binding
    }
}
