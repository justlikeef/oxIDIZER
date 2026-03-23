use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, HtmlInputElement, HtmlElement, HtmlSelectElement, Event};
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
    let inputs = container.query_selector_all("input, select, textarea")?;
    
    for i in 0..inputs.length() {
        let el = inputs.item(i).expect("index out of range").dyn_into::<Element>()?;
        
        let container_id_owned = container_id.to_string();
        
        // Listener for conditions
        let handler = Closure::wrap(Box::new(move |_: Event| {
            if let Err(e) = update_conditions(&container_id_owned) {
                web_sys::console::error_1(&e);
            }
        }) as Box<dyn FnMut(_)>);

        el.add_event_listener_with_callback("change", handler.as_ref().unchecked_ref())?;
        el.add_event_listener_with_callback("input", handler.as_ref().unchecked_ref())?;
        handler.forget(); 
    }

    init_conditions(container_id)?;

    Ok(())
}

fn init_conditions(container_id: &str) -> Result<(), JsValue> {
    update_conditions(container_id)
}

fn update_conditions(container_id: &str) -> Result<(), JsValue> {
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let container = document.get_element_by_id(container_id).unwrap();

    let cond_fields = container.query_selector_all(".conditional-field, .conditional-form")?;
    
    for i in 0..cond_fields.length() {
        let el = cond_fields.item(i).unwrap().dyn_into::<Element>()?;
        let condition = el.get_attribute("data-condition").unwrap_or_default();
        
        if condition.is_empty() { continue; }

        let is_visible = evaluate_condition(&condition, &container);
        
        let style = el.dyn_into::<web_sys::HtmlElement>()?.style();
        if is_visible {
            style.set_property("display", "block")?;
        } else {
            style.set_property("display", "none")?;
        }
    }

    Ok(())
}

fn evaluate_condition(condition: &str, container: &Element) -> bool {
    // Basic parser for "field == 'value'" or "field != 'value'"
    let parts: Vec<&str> = condition.split_whitespace().collect();
    if parts.len() < 3 { return true; }

    let field_name = parts[0];
    let op = parts[1];
    let raw_val = parts[2..].join(" ");
    let target_val = raw_val.trim_matches(|c| c == '\'' || c == '"');

    // Find the field value in container
    let selector = format!("[name='{}']", field_name);
    let field = match container.query_selector(&selector) {
        Ok(Some(f)) => f,
        _ => return false,
    };

    let current_val = if let Ok(input) = field.clone().dyn_into::<HtmlInputElement>() {
        match input.type_().as_str() {
            "checkbox" => if input.checked() { "true".to_string() } else { "false".to_string() },
            "radio" => {
                // Find checked radio button with same name
                let radios = container.query_selector_all(&format!("input[type='radio'][name='{}']", field_name)).unwrap();
                let mut val = String::new();
                for i in 0..radios.length() {
                    let r = radios.item(i).unwrap().dyn_into::<HtmlInputElement>().unwrap();
                    if r.checked() {
                        val = r.value();
                        break;
                    }
                }
                val
            },
            _ => input.value()
        }
    } else if let Ok(select) = field.clone().dyn_into::<web_sys::HtmlSelectElement>() {
        select.value()
    } else {
        String::new()
    };

    match op {
        "==" => current_val == target_val,
        "!=" => current_val != target_val,
        _ => true
    }
}

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
