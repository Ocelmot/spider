
use spider_link::{
    message::UiPage,
};

use super::{
    message::{SettingCategory, SettingType},
    UiProcessorState,
};

impl UiProcessorState {
    async fn init_settings(&mut self) {
        let id = self.state.self_id().await;
        let page = UiPage::new(id, "Settings");
        self.pages.upsert_page(page);
    }

    async fn add_setting(&mut self, category: SettingCategory, name: String, setting_type: SettingType) {
        

    }
}
