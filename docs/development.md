# Spider dev setup

[Return to README](README.md) \
[Peripheral Development](#peripheral-development) \
[Base Development](#base-development)

## Peripheral Development

This section covers what is required to develop your own peripheral for the
spider platform. At the moment the only development language is Rust. This guide
will assume that Rust is installed.

There are several test peripherals that were used in the development of the
platform which could be useful as examples of peripheral code. They are linked
here roughly in order of complexity.

- [Test Dataset](https://github.com/Ocelmot/spider_test_dataset)
- [Soil Probe](https://github.com/Ocelmot/spider_soil_probe)
- [Test Peripheral](https://github.com/Ocelmot/spider_test_peripheral)
- [Synapse](https://github.com/Ocelmot/Synapse)

### Setup

The only crate directly needed to create a peripheral is the spider_client
crate. However, this project uses tokio and exposes an async api, therefore
Tokio is also needed to create a peripheral. Include the following in your
cargo.toml file

```toml
tokio = { version = "1", features = ["full", "tracing"] }
spider_client = { git = "https://github.com/Ocelmot/spider" }
```

### Making a connection
The connection to the base is handled by the ClientChannel. This is created via
a builder, SpiderClientBuilder. The client automatically stores its state in a
file that it will try to load on subsequent launches. This state include things
like its id and connection preferences.

``` Rust
use spider_client::SpiderClientBuilder;

// Path to where the state is or will be stored.
let client_path = PathBuf::from("client_state.dat");

// Loads settings from the state file, or creates a new configuration.
// If a new configuration is created, it will be passed to the function.
let mut builder = SpiderClientBuilder::load_or_set(&client_path, |builder| {
    // The following causes the connection to look for the base on localhost.
    builder.enable_fixed_addrs(true);
    builder.set_fixed_addrs(vec!["localhost:1930".into()]);
});

// Trying to use the keyfile. The keyfile allows the peripheral to pair with
// the base automatically, when the peripheral is launched as a service.
// This does nothing if the file does not exist.
builder.try_use_keyfile("spider_keyfile.json").await;

// Turns the builder into the client channel.
// The first parameter is a boolean that indicates if the channel should also
// recieve messages from the base.
let client_channel = builder.start(true);
```

### Processing Input and Output

Once the client channel is connected, it can be used to send and recieve
messages to and from the base. Here is a quick example to how how messages are sent to the base.

``` Rust
// Set the name of this peripheral as seen in the base's settings menu
let msg = RouterMessage::SetIdentityProperty("name".into(), "Synapse".into());
// Wrap the message in the outer layer
let msg = Message::Router(msg);
// Send the message down the channel
self.client.send(msg).await;
```

Conversely, this is an example of how input messages could be handled.

``` Rust
// Loop to continually recieve messages
loop {
    // Recieve a message from the client channel
    match client_channel.recv().await {
        // If a message about the UI was recieved, process it with ui_handler
        Some(ClientResponse::Message(Message::Ui(msg))) => {
            ui_handler(msg).await,
        }
        // If a message about datasets is recieved, process with dataset_handler
        Some(ClientResponse::Message(Message::Dataset(msg))) => {
            dataset_handler(msg).await
        }
        // If a message about routing was recieved, process with router_handler
        Some(ClientResponse::Message(Message::Router(msg))) => {
            router_handler(msg).await
        }
        // If no message is recieved, the connection is done
        None => break, //  done!
        // Ignore unneeded messages
        _ => {}
    }
}
```

The `ui_handler`, `dataset_handler`, and `router_handler` then further break the recieved message apart and handle the more specific cases. For a more detailed example, take a look at some of the linked example projects above.

### Setting up a UI Page

The base also provides a simple UI functionality to its connected peripherals. First the peripheral builds a UIElement tree to indicate what its UI page should look like. This is a simple example of how to build the page.

``` Rust
// Create a new page manager with the peripheral's id.
let id = client_channel.id().clone();
let mut page_mgr = UiPageManager::new(id, "Test Page");

// Get the root element from the page manager
let mut root = test_page
    .get_element_mut(&UiPath::root())
    .expect("all pages have a root");

// Modify the root element to display the desired page.
// The root should arrange its children as rows
root.set_kind(UiElementKind::Rows);

// Add a static string to the page
root.append_child(UiElement::from_string("Value is: "));

// Add a placeholder data element to the page
root.append_child({
    let mut element = UiElement::from_string("0");
    // Set the id of this element so that it can be referred to later.
    element.set_id("data");
    element
});

// Finally, add a row with two buttons to control the data item.
root.append_child({
    // The buttons should be arranged horizontally
    let mut child = UiElement::new(UiElementKind::Columns);

    // Increase button
    child.append_child({
        // Set the text on this button
        let mut element = UiElement::from_string("Increase");
        // Change the kind of this element to button
        element.set_kind(UiElementKind::Button);
        // Buttons should be selectable
        element.set_selectable(true);
        // Set the id of the button so that its inputs are clear
        element.set_id("increase");
        // Return the element
        element
    });

    // Decrease button
    child.append_child({
        let mut element = UiElement::from_string("Decrease");
        element.set_kind(UiElementKind::Button);
        element.set_selectable(true);
        element.set_id("decrease");
        element
    });
    
    // Return the row of buttons
    child
});

// Drop the reference to the root of the page so that the page manager
// can be used mutably.
drop(root);

// This clears any changes that have been made during initialization since
// the entire page is sent initially. This will be used later.
page_mgr.get_changes();
```

Next, the UI page must be sent to the base.

``` Rust
// Clone the page and send it to the base
let msg = Message::Ui(UiMessage::SetPage(page_mgr.get_page().clone()));
client_channel.send(msg).await;
```

If the UI has updated a portion of the page, there is no need to resend the
whole page. It is possible to collect changes to the UIPage since its last
modification, and only send those changes. This should help with larger or
frequently updated pages.

This is an excerpt from the ui_handler function in the example project called
[Test Peripheral](https://github.com/Ocelmot/spider_test_peripheral)

``` Rust
async fn ui_handler(client: &mut ClientChannel, state: &mut State, msg: UiMessage) {
    match msg {
        // Respond to input events
        UiMessage::Input(element_id, _, change) => {
            // Test the id of the source of the event
            match element_id.as_str() {
                "increase" => {
                    // Get the element used to display the data
                    let mut element = state.page_mgr.get_by_id_mut("data").unwrap();
                    // Do some calculations to make the desired modifications
                    state.page_num += 1;
                    // Modify the element to reflect the new data
                    element.set_text(format!("{}", state.page_num));
                }
                "decrease" => {
                    // Get the element used to display the data
                    let mut element = state.page_mgr.get_by_id_mut("data").unwrap();
                    // Do some calculations to make the desired modifications
                    state.page_num = state.page_num.saturating_sub(1);
                    // Modify the element to reflect the new data
                    element.set_text(format!("{}", state.page_num));
                }
                // If the element id is unknown do nothing
                _ => return,
            }
        }
    }

    // Get the changes from the page manager
    let changes = state.page_mgr.get_changes();
    // Only send the changes rather than the whole page
    let msg = Message::Ui(UiMessage::UpdateElements(changes));
    client.send(msg).await;
}
```

Finally, if the peripheral no longer requires or wishes to maintain its UI page,
it can clear that page to remove it from the list of pages in the base.

``` Rust
let msg = Message::Ui(UiMessage::ClearPage);
client_channel.send(msg).await;
```

### Using Datasets
The base provides storage for the peripherals in datasets. These datasets hold a
sequence of data objects, similar to a JSON array. Datasets are differentiated
from each other using a path system similar to paths used by the local
filesystem. The base maintains a set of datasets that are public and can be
shared by all peripherals. It also maintains a set of datasets that are private
and unique to each connected peripheral.

To add an element to a dataset send something like the following

``` Rust
// Create a new DatasetPath from a vec of parts
let dataset_path = DatasetPath::new_private(vec![String::from("Test")]);

// Create the data to be added
let data = spider_client::message::DatasetData::String(String::from(
    "added data!",
));

// Build and send the message.
// This will append the new data to the end of the dataset.
let msg = Message::Dataset(DatasetMessage::Append {
    path: dataset_path,
    data: data,
});
client_channel.send(msg).await;
```

A similar process to delete the data

``` Rust
// Create a new DatasetPath from a vec of parts
let dataset_path = DatasetPath::new_private(vec![String::from("Test")]);

// Build the message to delete the element.
// This will delete the first element (index 0).
let msg = Message::Dataset(DatasetMessage::DeleteElement {
    path: dataset_path,
    id: 0, // Delete first element
});
client_channel.send(msg).await;
```


One additional feature that makes datasets extremely powerful is how they
integrate with the UI pages. A UIElement may indicate that its child elements
should be repeated once for each element in a dataset. When this is the case,
the child elements can then refer to the properties of the object it corresponds
to and the referenced data will be rendered in the UI. Furthermore, this means
that adding an item only means appending to the corresponding dataset. The UI
structure itself does not have to be changed.

Setup the Ui like this:

``` Rust
// Append a new child to the root element
root.append_child({
    // The element is a set of rows, the items from the dataset will
    // render vertically.
    let mut element = UiElement::new(UiElementKind::Rows);

    // This element's children will be replicated
    // once for each item in the dataset.
    element.set_dataset(Some(dataset_path.clone().resolve(id.clone())));

    // child element
    element.append_child({
        // Arranged horizontally
        let mut child = UiElement::new(UiElementKind::Columns);

        // child to render the data item
        child.append_child({
            let mut child = UiElement::new(UiElementKind::Text);
            let mut content = UiElementContent::new();
            // Empty vec indicates that the entire dataset item will be
            // rendered here. The elements of the vec are used to index into
            // the dataset item.
            content.add_part(UiElementContentPart::Data(vec![]));
            // Set this content to be the content of this item
            child.set_content(content);
            child
        });
        
        // Add space between the data element and the controls
        child.append_child(UiElement::new(UiElementKind::Spacer));

        // Add some controls to be able to delete an item
        child.append_child({
            let mut child = UiElement::new(UiElementKind::Button);
            child.set_id("delete row");
            child.set_selectable(true);
            child.set_text("Delete!");
            child
        });
        child
    });
    element
});
```

As items are added or removed from the dataset, the UI will automatically change without having to be relayed through the peripher process itself.

## Base Development

This section covers the small amount of additional setup to begin development
for the base portion of the spider platform. If you are just starting out and
want to make something that uses the platform, the [Peripheral
Development](#peripheral-development) section above is where you should start. 

To start, follow the installation directions from the main page
([here](README.md#requirements)). Stop when you reach the `Build / Run` section.
Add the following to the end of the the cargo toml file
`<project_root>/.cargo/config.toml`.
``` toml
[patch.'https://github.com/Ocelmot/spider']
spider = { path = "spider_base/spider" }
spider_link = { path = "spider_base/spider_link" }
spider_client = { path = "spider_base/spider_client" }
```
This will make sure that this local copy will be used when compiling instead of the copy that cargo will make as part of its dependency management. Changes made to the code in the `<project_root>/spider_base` directory will be reflected in both the base and the client when they are recompiled.
