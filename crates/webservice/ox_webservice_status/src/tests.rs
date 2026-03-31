use crate::{ox_plugin_init, ox_plugin_process};
use ox_webservice_test_utils::{
    create_mock_api, create_task_state, drop_task_state, get_mock_field, set_mock_field, PluginHandle,
};
use ox_workflow_abi::FLOW_CONTROL_CONTINUE;

#[test]
fn test_status_json_accept() {
    let api = create_mock_api();
    let handle = PluginHandle::init(ox_plugin_init, "{}", &api).expect("Failed to load status module");

    let task_ctx = create_task_state();
    set_mock_field(task_ctx, "request.header.Accept", "application/json");
    let flow = handle.process(ox_plugin_process, task_ctx);
    assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);

    let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
    assert!(body.starts_with('{'), "Expected JSON body, got: {}", body);

    let ct = get_mock_field(task_ctx, "response.header.Content-Type").unwrap_or_default();
    assert_eq!(ct, "application/json");

    unsafe { drop_task_state(task_ctx); }
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_status_json_query() {
    let api = create_mock_api();
    let handle = PluginHandle::init(ox_plugin_init, "{}", &api).expect("Failed to load status module");

    let task_ctx = create_task_state();
    set_mock_field(task_ctx, "request.format", "json");
    let flow = handle.process(ox_plugin_process, task_ctx);
    assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);

    let body = get_mock_field(task_ctx, "response.body").unwrap_or_default();
    assert!(body.starts_with('{'), "Expected JSON body, got: {}", body);

    unsafe { drop_task_state(task_ctx); }
}
