use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, HtmlInputElement, Event};
use serde_json::Value;

#[wasm_bindgen]
pub fn init_client() {
    console_error_panic_hook::set_once();
    web_sys::console::log_1(&"ox_forms_client initialized".into());
}

#[wasm_bindgen]
pub fn attach_listeners(container_id: &str) -> Result<(), JsValue> {
    let window = web_sys::window().expect("no global `window` exists");
    let document = window.document().expect("should have a document on window");
    
    let container = document.get_element_by_id(container_id)
        .ok_or_else(|| JsValue::from_str("Container not found"))?;

    // Find all inputs that need validation or binding
    // For this prototype, we'll look for inputs with `data-ox-validation`
    let inputs = container.query_selector_all("input")?;
    
    for i in 0..inputs.length() {
        let input = inputs.item(i).expect("index out of range");
        let input_el = input.dyn_into::<HtmlInputElement>()?;
        
        let name = input_el.name();
        
        // Closure needs to be static or long-lived. 
        // For simplicity in this demo, we attach a one-off handler that logs.
        // real implementation would manage closure lifetime or use a central event delegation.
        
        let handler = Closure::wrap(Box::new(move |event: Event| {
            let target = event.target().unwrap();
            let el = target.dyn_into::<HtmlInputElement>().unwrap();
            web_sys::console::log_1(&format!("Input changed: {} = {}", el.name(), el.value()).into());
            
            // Here we would call into shared ox_forms::validate(field_def, value)
            // But we need the field definition on the client side.
            // Usually this means passing the FormDefinition JSON to the client.
        }) as Box<dyn FnMut(_)>);

        input_el.add_event_listener_with_callback("input", handler.as_ref().unchecked_ref())?;
        handler.forget(); // Leak memory for now to keep handler alive
    }

    Ok(())
}

use wasm_bindgen::closure::Closure;
