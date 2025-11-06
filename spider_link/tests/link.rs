use std::collections::HashMap;

use spider_link::message::{AbsoluteDatasetPath, DatasetData, UiElement, UiElementKind};
use tracing::info;

#[test]
fn test_ui_element_dataset_iterator() {
    let mut data_map: HashMap<AbsoluteDatasetPath, Vec<DatasetData>> = HashMap::new();

    let path = AbsoluteDatasetPath::new_public(vec!["test".into()]);
    let mut dataset = vec![
        DatasetData::String("data 1".into()),
        DatasetData::String("data 2".into()),
    ];
    dataset.push(DatasetData::String("data 3".into()));
    data_map.insert(path.clone(), dataset);

    let mut elem = UiElement::new(UiElementKind::Rows);
    elem.append_child(UiElement::from_string("Child 1"));

    elem.set_dataset(Some(path.clone()));

    info!("===== plain iteration =====");
    for (dataset_index, child, datum) in elem.children_dataset(&None, &data_map) {
        info!("idx: {:?}", dataset_index);
        info!("child:{:?}", child.render_content_opt(&datum));
        info!("datum: {:?}", datum);
    }

    info!("===== take 1 =====");
    for (dataset_index, child, datum) in elem.children_dataset(&None, &data_map).take(1) {
        info!("idx: {:?}", dataset_index);
        info!("child:{:?}", child.render_content_opt(&datum));
        info!("datum: {:?}", datum);
    }

    info!("===== collect =====");
    let mut iter = elem.children_dataset(&None, &data_map).take(1).rev();
    info!("{:?}", iter.size_hint());
    info!("{:?}", iter.len());
    info!("{:?}", iter.next_back());

    info!("===== manual =====");
    let iter = elem.children_dataset(&None, &data_map);
    info!("len: {:?}", iter.len());
    info!("size_hint: {:?}", iter.size_hint());
    info!("count {:?}", iter.count());
}
