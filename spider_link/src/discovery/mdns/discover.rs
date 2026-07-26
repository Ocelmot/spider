




pub struct MdnsDiscoverer{
    browser: MdnsBrowserAsync,

}

impl MdnsDiscoverer{
    fn new() -> Self{
        let mut browser = MdnsBrowserAsync::new(MdnsBrowser::new(service_type))?;
        browser.start().await?;
        Self{}
    }
}

impl Discoverer for MdnsDiscoverer {
    fn next_addr(&mut self) -> Pin<Box<dyn Future<Output = AdvertEvent> + Send + '_>> {
        let fut = async {
            while let Some(event) = self.browser.next.await {
                match event {
                    BrowerEvent::Add(discovery) => {

                    },
                    BrowserEvent::Remove(removal) => {
                        
                    }
                }
            }
        };
        Box::pin(fut);
    }
}
