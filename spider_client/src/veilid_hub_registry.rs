use std::{collections::BTreeMap, future::Future, ops::DerefMut, pin::Pin, sync::Arc};

use spider_link::{
    link::{VeilidConnector, VeilidHub, VeilidLink},
    SelfRelation,
};
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};
use veilid_core::{DHTRecordDescriptor, VeilidConfigInner};

use crate::error::{ClientResult, ProblemWrap};

static VEILID_REGISTRY: Mutex<BTreeMap<SelfRelation, RegistryEntry>> =
    Mutex::const_new(BTreeMap::new());

type CallbackFunction =
    dyn FnMut(VeilidLink) -> Pin<Box<dyn Future<Output = ()> + Send + Sync>> + Send + Sync;

pub(crate) struct RegistryEntry {
    hub: VeilidHub,
    listener_callback: Arc<Mutex<Option<Box<CallbackFunction>>>>,
}

impl RegistryEntry {
    async fn new(
        listen_rel: SelfRelation,
        listen_dht: Option<DHTRecordDescriptor>,
        config: VeilidConfigInner,
    ) -> ClientResult<Self> {
        let (hub, mut listen_channel) = VeilidHub::new(listen_rel, listen_dht, config)
            .await
            .wrap()?;
        let listener_callback = Arc::new(Mutex::new(None::<Box<CallbackFunction>>));

        let task_callback = listener_callback.clone();
        tokio::spawn(async move {
            loop {
                let Some(link) = listen_channel.recv().await else {
                    return;
                };
                let mut guard = task_callback.lock().await;
                if let Some(ref mut cb) = guard.deref_mut() {
                    cb(link).await;
                }
            }
        });

        Ok(Self {
            hub,
            listener_callback,
        })
    }

    pub(crate) async fn get_or_create_entry(
        listen_rel: SelfRelation,
        listen_dht: Option<DHTRecordDescriptor>,
        config: VeilidConfigInner,
    ) -> ClientResult<MappedMutexGuard<'static, RegistryEntry>> {
        let mut registry = VEILID_REGISTRY.lock().await;

        if !registry.contains_key(&listen_rel) {
            let new_entry = RegistryEntry::new(listen_rel.clone(), listen_dht, config).await?;
            registry.insert(listen_rel.clone(), new_entry);
        }
        Ok(MutexGuard::map(registry, |registry| {
            registry
                .get_mut(&listen_rel)
                .expect("registry should have had an entry added if it did not already exist")
        }))
    }

    pub(crate) async fn get_entry(listen_rel: &SelfRelation) -> Option<MappedMutexGuard<'static, RegistryEntry>> {
        let registry = VEILID_REGISTRY.lock().await;
        MutexGuard::try_map(registry, |registry|{
            registry.get_mut(listen_rel)
        }).ok()
    }

    pub(crate) fn get_listen_dht(&self) -> &DHTRecordDescriptor {
        self.hub.listen_dht()
    }

    pub(crate) fn get_connector(&self) -> VeilidConnector {
        self.hub.get_connector()
    }

    pub(crate) async fn set_listener_callback<Func, Fut>(&mut self, mut cb: Func)
    where
        Func: FnMut(VeilidLink) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + Sync + 'static,
    {
        let mut guard = self.listener_callback.lock().await;
        *guard = Some(Box::new(move |link| Box::pin(cb(link))));
    }
    pub(crate) async fn clear_listener_callback(&mut self) {
        let mut guard = self.listener_callback.lock().await;
        *guard = None;
    }
    pub(crate) async fn has_listener_callback(&self) -> bool {
        let guard = self.listener_callback.lock().await;
        guard.is_some()
    }
}
