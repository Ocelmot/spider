
use std::collections::HashMap;

use spider_link::{
    message::{UiPage, UiElementKind, UiPath, UiMessage, UiElement, UiInput, DatasetPath, UiElementContent, UiElementContentPart, DatasetData, DatasetMessage},
};

use crate::processor::{dataset::DatasetProcessorMessage, message::ProcessorMessage};

use super::{
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

    pub(crate) async fn add_setting(&mut self, header: String, title: String, inputs: Vec<(String, String)>, cb: fn(u32, &String, UiInput)->Option<ProcessorMessage>) {
        let id = self.state.self_id().await;

        // find header
        // if no header, must insert
        let mgr = self.pages.get_page_mut(&id).expect("page should still exist");
        match mgr.get_by_id(&header){
            Some(_) => {},
            None => {
                // dataset path
                let dataset_path = DatasetPath::new_private(vec!["settings".to_string(), header.clone()]);
                let abs_dataset_path = dataset_path.clone().resolve(id.clone());

                // create header element, and insert
                let mut elem = UiElement::new(UiElementKind::Rows);
                elem.set_id(header.clone());

                elem.append_child(UiElement::from_string(header.clone()));
                elem.append_child({
                    // list of settings, matches to a dataset
                    let mut settings_list_element = UiElement::new(UiElementKind::Rows);
                    settings_list_element.set_dataset(Some(abs_dataset_path.clone()));

                    settings_list_element.append_child({
                        let mut row = UiElement::new(UiElementKind::Columns);

                        row.append_child({
                            let mut title = UiElement::new(UiElementKind::Text);
                            title.set_content(UiElementContent::new_data("title".to_string()));
                            title
                        });
                        row.append_child(UiElement::new(UiElementKind::Spacer));
                        row.append_child({
                            let input_type = UiElementKind::Variable(UiElementContentPart::Data(vec!["input_0_type".to_string()]));
                            let mut input = UiElement::new(input_type);
                            input.set_id(format!("{}0", header.clone()));
                            input.set_selectable(true);
                            input.set_content(UiElementContent::new_data("input_0_label".to_string()));
                            input
                        });
                        row.append_child({
                            let input_type = UiElementKind::Variable(UiElementContentPart::Data(vec!["input_1_type".to_string()]));
                            let mut input = UiElement::new(input_type);
                            input.set_id(format!("{}1", header.clone()));
                            input.set_selectable(true);
                            input.set_content(UiElementContent::new_data("input_1_label".to_string()));
                            input
                        });
                        row.append_child({
                            let input_type = UiElementKind::Variable(UiElementContentPart::Data(vec!["input_2_type".to_string()]));
                            let mut input = UiElement::new(input_type);
                            input.set_id(format!("{}2", header.clone()));
                            input.set_selectable(true);
                            input.set_content(UiElementContent::new_data("input_2_label".to_string()));
                            input
                        });

                        row
                    });

                    settings_list_element
                });

                let mut root = mgr.get_element_mut(&UiPath::root()).expect("all pages have a root");
                root.append_child(elem);
                drop(root);



                // send to clients here
                let updates = mgr.get_changes();
                let msg = UiMessage::UpdateElementsFor(
                    id.clone(),
                    updates,
                );
                self.ui_to_subscribers(msg).await;
                
                // clear any old data in the settings area
                let rel = self.state.self_relation().await.relation;
                self.sender.send_dataset(DatasetProcessorMessage::PublicMessage(rel, DatasetMessage::Empty { path: dataset_path.clone() })).await;
                
                // update subscriptions
                self.update_dataset_subscriptions(&abs_dataset_path, 1).await;
            },
        };

        
        // Create Dataset Item
        let null_inputs = vec![("none".to_string(), String::new()); 3];
        let mut map = HashMap::new();
        map.insert("header".to_string(), DatasetData::String(header.clone()));
        map.insert("title".to_string(), DatasetData::String(title.clone()));
        for (index, (input_type, input_label)) in inputs.into_iter().chain(null_inputs.into_iter()).take(3).enumerate(){
            map.insert(format!("input_{}_type", index), DatasetData::String(input_type));
            map.insert(format!("input_{}_label", index), DatasetData::String(input_label));
        }
        let data_item = DatasetData::Map(map);
        let dataset_path = DatasetPath::new_private(vec!["settings".to_string(), header.clone()]);
        let rel = self.state.self_relation().await.relation;

        

        // Register input callback
        match self.settings_callbacks.get_mut(&header){
            Some((title_map, list)) => {
                // test if title exists
                match title_map.get(&title){
                    Some(index) => {
                        // Index exists, update existing
                        list[*index] = (title.clone(), cb);

                        // update dataset item
                        let dataset_message = DatasetMessage::SetElement { path: dataset_path, data: data_item, id: *index };
                        let dataset_processor_message = DatasetProcessorMessage::PublicMessage(rel, dataset_message);
                        self.sender.send_dataset(dataset_processor_message).await;
                    },
                    None => {
                        // Index does not exist, push to back
                        title_map.insert(title.clone(), list.len());
                        list.push((title.clone(), cb));

                        // Insert dataset item
                        self.sender.send_dataset(DatasetProcessorMessage::PublicMessage(rel, DatasetMessage::Append { path: dataset_path, data: data_item })).await;
                    },
                }
            },
            None => {
                // save callback
                let title_map = HashMap::new();
                let list = vec![(title.clone(), cb)];
                self.settings_callbacks.insert(header, (title_map, list));

                // Insert dataset item
                self.sender.send_dataset(DatasetProcessorMessage::PublicMessage(rel, DatasetMessage::Append { path: dataset_path, data: data_item })).await;
            },
        }       
    }

    pub(crate) async fn remove_setting(&mut self, header: String, title: String,){
        match self.settings_callbacks.get_mut(&header){
            Some((title_map, list)) => {
                match title_map.remove(&title){
                    Some(index) => {
                        list.remove(index);
                        // must update all entries in title map that are greater than index
                        // it might be better to do this when the dataset updates
                        for (_, map_index) in title_map{
                            if *map_index > index {
                                *map_index -= 1;
                            }
                        }

                        // remove from dataset as well
                        let dataset_path = DatasetPath::new_private(vec!["settings".to_string(), header.clone()]);
                        let rel = self.state.self_relation().await.relation;

                        let dataset_message = DatasetMessage::DeleteElement { path: dataset_path, id: index };
                        let dataset_processor_message = DatasetProcessorMessage::PublicMessage(rel, dataset_message);
                        self.sender.send_dataset(dataset_processor_message).await;
                    },
                    None => {}, // no title, nothing to remove
                }
            },
            None => {}, // No header, nothing to remove
        }
    }

    pub(crate) async fn settings_input(&mut self, element_id: &String, dataset_ids: Vec<usize>, input: UiInput){
        let mut element_id = element_id.to_string();
        let input_index = match element_id.pop(){
            Some(elem) => {
                match elem.to_digit(10){
                    Some(num) => num,
                    None => return, // invalid id char
                }
            },
            None => return, // chars in id, return
        };
        let header = element_id;

        match self.settings_callbacks.get_mut(&header){
            Some((x, list)) => {
                let func_index = dataset_ids.last().unwrap(); // get innermost dataset id
                match list.get_mut(*func_index) {
                    Some((title, func)) => {
                        let msg = func(input_index, title, input);
                        match msg{
                            Some(msg) => {
                                self.sender.send(msg).await;
                            },
                            None => {},
                        }
                    },
                    None => {
                        // No such function in list
                    },
                }
            },
            None => {
                // settings list is missing (ignore)
            },
        }
    }
}
