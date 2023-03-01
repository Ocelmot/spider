use spider_link::{Relation, message::UiMessage};




#[derive(Debug)]
pub enum UiProcessorMessage{
    RemoteMessage(Relation, UiMessage),
    SetSetting{category: SettingCategory, name: String, setting_type: SettingType},
    Upkeep,
}

#[derive(Debug)]
pub enum SettingCategory{
    Test,
}

#[derive(Debug)]
pub enum SettingType{
    String,
    Toggle,
}