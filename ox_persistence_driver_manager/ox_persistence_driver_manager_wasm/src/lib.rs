use yew::prelude::*;
use serde::{Deserialize, Serialize};
use reqwasm::http::Request;
use wasm_bindgen::prelude::*;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ConfiguredDriver {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub library_path: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct DriversList {
    #[serde(default)]
    pub drivers: Vec<ConfiguredDriver>,
}

#[function_component(App)]
pub fn app() -> Html {
    let drivers = use_state(|| vec![]);
    let error = use_state(|| Option::<String>::None);
    
    {
        let drivers = drivers.clone();
        let error = error.clone();
        use_effect_with((), move |_| {
            let drivers = drivers.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let fetched_drivers: Result<DriversList, _> = Request::get("/drivers/")
                    .header("Accept", "application/json")
                    .send()
                    .await
                    .unwrap()
                    .json::<DriversList>()
                    .await;

                match fetched_drivers {
                    Ok(list) => drivers.set(list.drivers),
                    Err(e) => error.set(Some(format!("Failed to fetch drivers: {}", e))),
                }
            });
            || ()
        });
    }

    let toggle_status = {
        let drivers = drivers.clone();
        let error = error.clone();
        
        Callback::from(move |id: String| {
            let drivers = drivers.clone();
            let error = error.clone();
            
            wasm_bindgen_futures::spawn_local(async move {
                let url = format!("/drivers/{}", id);
                let response = Request::post(&url)
                    .header("Accept", "application/json")
                    .send()
                    .await;
                    
                match response {
                    Ok(resp) => {
                        if resp.status() == 200 {
                            // Refresh list
                             let fetched: Result<DriversList, _> = Request::get("/drivers/")
                                .header("Accept", "application/json")
                                .send()
                                .await
                                .unwrap()
                                .json::<DriversList>()
                                .await;
                             
                             if let Ok(list) = fetched {
                                 drivers.set(list.drivers);
                             }
                        } else {
                            error.set(Some(format!("Failed to toggle status: {}", resp.status_text())));
                        }
                    },
                    Err(e) => error.set(Some(format!("Network error: {}", e))),
                }
            });
        })
    };

    html! {
        <div class="container">
            <header>
                 <div style="display: flex; justify-content: space-between; align-items: flex-start;">
                    <div style="display: flex; flex-direction: column; align-items: flex-start;">
                        <img src="/images/logo.png" alt="oxIDIZER" style="height: 6rem; margin-bottom: 0.5rem; filter: drop_shadow(0 0 5px rgba(0,0,0,0.5));" />
                        <h1 style="font-size: 3rem; line-height: 1.1;">{ "Persistence Driver Manager" }</h1>
                    </div>
                    <div class="version-badge" style="background: rgba(0,0,0,0.1); padding: 0.5rem 1rem; border-radius: 4px; font-weight: 600; color: var(--text-secondary);">
                        { "WASM" }
                    </div>
                </div>
            </header>

            <div class="section-title">{ "Configured Drivers" }</div>
            
            if let Some(err) = &*error {
                <div class="error-banner" style="background: #e74c3c; color: white; padding: 1rem; margin-bottom: 1rem; border-radius: 4px;">
                    { err }
                </div>
            }

            <div class="grid">
                { for drivers.iter().map(|driver| {
                    let status_class = if driver.state == "enabled" { "status-ok" } else { "status-error" };
                    let id_clone = driver.id.clone();
                    let toggle = toggle_status.clone();
                    
                                    let lib_name = format!("lib{}.so", &driver.name);
                                    let dir_path = if driver.library_path.is_empty() {
                                        "target/debug"
                                    } else {
                                        &driver.library_path
                                    };
                                    let tooltip = format!("{}/{}", dir_path, lib_name);
                                    
                                    html! {
                                        <div class={classes!("card", status_class)}>
                                            <div class="card-header">
                                                <div class={classes!("status-badge", status_class)}>{ &driver.state.to_uppercase() }</div>
                                                <div class="card-title">{ &driver.id }</div>
                                            </div>
                                            <div class="card-content">
                                                 <div class="kv-row">
                                                    <span class="kv-key">{ "Library" }</span>
                                                    <span class="kv-value" title={tooltip}>{ lib_name }</span>
                                                </div>
                                <div class="actions" style="margin-top: 1rem; text-align: right;">
                                    <button onclick={move |_| toggle.emit(id_clone.clone())} class={classes!("btn", "toggle-btn", if driver.state == "enabled" { "btn-disable" } else { "btn-enable" })}>
                                        { if driver.state == "enabled" { "Disable" } else { "Enable" } }
                                    </button>
                                </div>
                            </div>
                        </div>
                    }
                }) }
            </div>
        </div>
    }
}

#[wasm_bindgen(start)]
pub fn run_app() {
    yew::Renderer::<App>::new().render();
}
