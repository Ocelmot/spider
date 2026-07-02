//! Implementation of Connector and Listener for the Iroh protocol

use std::str::FromStr;

pub use iroh::SecretKey;
use iroh::{
    endpoint::{presets, Builder, QuicTransportConfig, QuicTransportConfigBuilder},
    Endpoint,
};

use link_set::links::LinkConnector;
use tokio::{sync::mpsc::Sender, task::JoinHandle};
use tracing::debug;

use crate::{
    error::ProblemWrap,
    link_impls::{
        authenticated::{connect_link_attested, Authenticated},
        iroh_link::IrohLink,
    },
    transports::LinkListener,
    LinkError, LinkResult, Relation, SelfRelation,
};

/// The string tag to indicate the Iroh connector scheme
pub const IROH_SCHEME: &'static str = "iroh";
/// The ALPN used by Iroh
pub const ALPN: &[u8] = b"spider_transport/1";

/// Implementation of the [LinkConnector] trait for Iroh addresses/ids -> Links
pub struct IrohConnector {
    endpoint: Endpoint,
    sr: SelfRelation,
    rel: Relation,
}

impl IrohConnector {
    /// Create a new IrohConnector. This contains the endpoint to generate the
    /// iroh connection, as well as the relation and selfrelation required for
    /// authentication.
    pub fn new(sr: SelfRelation, rel: Relation, endpoint: Endpoint) -> Self {
        Self { endpoint, sr, rel }
    }
}

impl LinkConnector for IrohConnector {
    fn scheme(&self) -> &'static str {
        IROH_SCHEME
    }

    async fn connect(
        &mut self,
        addr: String,
    ) -> Result<impl link_set::links::Link + 'static, impl std::error::Error + Send + Sync + 'static>
    {
        let id = iroh::EndpointId::from_str(&addr).wrap()?;
        let conn = self
            .endpoint
            .connect(id, ALPN)
            .await
            .wrap_msg(format!("Failed to connect to iroh at {}", id))?;

        let link = IrohLink::new(conn);

        let (_rel, auth_link) =
            connect_link_attested(self.sr.clone(), self.rel.clone(), link).await?;

        LinkResult::Ok(auth_link)
    }
}

/// The hub that starts the Iroh process and can generate listeners
pub struct IrohHub {
    endpoint: Endpoint,
}

impl IrohHub {
    /// Create a new hub using the given secret and default presets.
    pub async fn new(secret: SecretKey) -> LinkResult<Self> {
        let builder = Endpoint::builder(presets::N0)
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec()]);
        Self::from_builder(builder).await
    }

    /// Create a new hub from an [iroh::endpoint::Builder].
    pub async fn from_builder(builder: Builder) -> LinkResult<Self> {
        let transport_builder = QuicTransportConfig::builder();
        Self::from_builders(builder, transport_builder).await
    }

    /// Create a new hub from an [iroh::endpoint::Builder].
    ///
    /// Enforces the datagram buffers size for the transport builder.
    pub async fn from_builders(
        mut builder: Builder,
        mut transport_builder: QuicTransportConfigBuilder,
    ) -> LinkResult<Self> {
        transport_builder = transport_builder.datagram_receive_buffer_size(Some(10_000_000));
        transport_builder = transport_builder.datagram_send_buffer_size(10_000_000);
        builder = builder.transport_config(transport_builder.build());
        let endpoint = builder.bind().await.wrap_msg("Failed to bind endpoint")?;
        Ok(Self { endpoint })
    }

    /// Gets a reference to the internal Iroh [Endpoint]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl LinkListener for IrohHub {
    fn scheme(&self) -> &'static str {
        IROH_SCHEME
    }

    fn listen(
        &self,
        sr: SelfRelation,
        sender: Sender<Authenticated>,
    ) -> JoinHandle<LinkResult<()>> {
        let endpoint = self.endpoint.clone();
        tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let conn = match incoming.await {
                    Ok(c) => c,
                    Err(e) => {
                        debug!("Iroh dropped incoming connection: {}", e);
                        continue;
                    }
                };

                // conn -> IrohLink
                let link = IrohLink::new(conn);
                let inner_sender = sender.clone();
                let inner_sr = sr.clone();
                tokio::spawn(async move {
                    // IrohLink -> Authenticated
                    match Authenticated::listen_attested(inner_sr, link).await {
                        Ok(authed) => {
                            inner_sender.send(authed).await.wrap()?;
                        }
                        Err(e) => {
                            debug!("Failed to authenticate Iroh link: {}", e);
                        }
                    }
                    Ok::<(), LinkError>(())
                });
            }
            LinkResult::Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use link_set::links::PinnedLink;
    use rand::random;
    use tokio::{join, sync::mpsc::channel};

    use super::*;

    async fn create_hub() -> IrohHub {
        let builder = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .alpns(vec![ALPN.to_vec()]);
        IrohHub::from_builder(builder).await.unwrap()
    }

    async fn simple_link_pair(
        hub_a: &IrohHub,
        listen_rel: &SelfRelation,
        hub_b: &IrohHub,
        connect_rel: &SelfRelation,
    ) -> LinkResult<(
        (Relation, Box<dyn PinnedLink>),
        (Relation, Box<dyn PinnedLink>),
    )> {
        let (tx_a, mut rx_a) = channel::<Authenticated>(10);
        hub_a.listen(listen_rel.clone(), tx_a);

        let a_addr = hub_a.endpoint().addr();

        let (connect_link, listen_link) = join!(async { rx_a.recv().await.unwrap() }, async {
            let conn = hub_b.endpoint().connect(a_addr, ALPN).await.unwrap();
            Authenticated::connect_attested(
                connect_rel.clone(),
                listen_rel.relation.clone(),
                IrohLink::new(conn),
            )
            .await
            .unwrap()
        });

        Ok((connect_link.into_parts(), listen_link.into_parts()))
    }

    #[tokio::test]
    async fn listen_connect_test() {
        let listen_hub = create_hub().await;
        let listen_rel = SelfRelation::debug_get(0);
        let connect_hub = create_hub().await;
        let connect_rel = SelfRelation::debug_get(1);

        let ((_, mut connect_link), (_, mut listen_link)) =
            simple_link_pair(&listen_hub, &listen_rel, &connect_hub, &connect_rel)
                .await
                .unwrap();

        let msg = b"test_data".to_vec();

        connect_link.send(msg.clone()).await.unwrap();

        let recvd_msg = listen_link.recv().await.unwrap();

        assert_eq!(msg, recvd_msg);
    }

    #[tokio::test]
    async fn max_size_test() {
        let listen_hub = create_hub().await;
        let listen_rel = SelfRelation::debug_get(0);
        let connect_hub = create_hub().await;
        let connect_rel = SelfRelation::debug_get(1);

        let ((_, mut connect_link), (_, mut listen_link)) =
            simple_link_pair(&listen_hub, &listen_rel, &connect_hub, &connect_rel)
                .await
                .unwrap();

        let mut msg = Vec::with_capacity(connect_link.max_size() as usize);
        msg.extend(std::iter::repeat_with(random::<u8>).take(connect_link.max_size() as usize));

        connect_link.send(msg.clone()).await.unwrap();

        let recvd_msg = listen_link.recv().await.unwrap();

        assert_eq!(msg, recvd_msg);
    }
}
