use ox_webservice_status::{ox_plugin_init, ox_plugin_process};
use ox_webservice_test_utils::{
    create_mock_api, create_task_state, drop_task_state, get_mock_field, set_mock_field, PluginHandle,
};
use serde_json::Value;

#[test]
#[cfg_attr(miri, ignore)]
fn test_info_leak() {
    let api = create_mock_api();
    let handle = PluginHandle::init(ox_plugin_init, "{}", &api).expect("init failed");

    let task_ctx = create_task_state();
    set_mock_field(task_ctx, "request.header.Accept", "application/json");
    let _ = handle.process(ox_plugin_process, task_ctx);

    if let Some(body) = get_mock_field(task_ctx, "response.body") {
        if !body.is_empty() {
            let json: Value = serde_json::from_str(&body).unwrap();
            if let Some(env) = json.get("environment") {
                let env_str = env.to_string();
                assert!(!env_str.contains("SECRET"), "Status module leaked SECRET env var");
                assert!(!env_str.contains("KEY"), "Status module leaked KEY env var");
            }
        }
    }

    unsafe { drop_task_state(task_ctx); }
}
