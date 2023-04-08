
use spider_link::{
    message::{UiPage, UiElementKind, UiPath, UiMessage, UiElement, UiInput},
};

use crate::processor::sender::ProcessorSender;

use super::{
    message::{SettingCategory, SettingType},
    UiProcessorState,
};

impl UiProcessorState {
    pub(crate) async fn init_settings(&mut self) {
        let id = self.state.self_id().await;
        let page = UiPage::new(id.clone(), "Settings");
        self.pages.upsert_page(page);
        
        let mgr = self.pages.get_page_mut(&id).expect("page should still exist");
        let mut root = mgr.get_element_mut(&UiPath::root()).expect("all pages should have a root");

        root.set_kind(UiElementKind::Rows);
        drop(root);
        mgr.get_changes(); // flush changes, since this will occur before UIs are connected, dont have to send anywhere.
    }

    pub(crate) async fn add_setting(&mut self, c: SettingCategory, n: String, st: SettingType, cb: fn(&mut ProcessorSender, UiInput)) {
        let id = self.state.self_id().await;

        let mgr = self.pages.get_page_mut(&id).expect("page should still exist");
        let mut root = mgr.get_element_mut(&UiPath::root()).expect("all pages should have a root");

        // Register input callback
        self.settings_callbacks.insert(n.clone(), cb);

        // maybe put into sections via category?
        // create input elements

        let row = match st{
            SettingType::Button => {
                let mut elem = UiElement::new(UiElementKind::Button);
                elem.set_id(n.clone());
                elem.set_text(n.clone());
                elem
            },
            SettingType::Text => todo!(),
        };
        root.append_child(row);
        
        drop(root);

        let updates = mgr.get_changes(); 

        // send to clients here
        let msg = UiMessage::UpdateElementsFor(
            id.clone(),
            updates,
        );
        self.ui_to_subscribers(msg).await;
    }

    pub(crate) async fn settings_input(&mut self, name: &String, input: UiInput){
        let func = match self.settings_callbacks.get(name){
            Some(func) => func,
            None => {return}, // no registered cb, return (Shouldnt happen?!)
        };
        func(&mut self.sender, input);
    }
}
