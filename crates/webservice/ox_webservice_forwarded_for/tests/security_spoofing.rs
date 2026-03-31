use ox_webservice_forwarded_for::{ox_plugin_init, ox_plugin_process};
use ox_webservice_test_utils::{
    create_mock_api, create_task_state, drop_task_state, get_mock_field, set_mock_field, PluginHandle,
};
use ox_workflow_abi::FLOW_CONTROL_CONTINUE;

#[test]
fn test_spoofing_resilience() {
    let api = create_mock_api();
    let handle = PluginHandle::init(ox_plugin_init, "{}", &api).expect("init failed");

    // Test 1: Multiple X-Forwarded-For IPs — module takes the leftmost
    {
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.header.X-Forwarded-For", "1.2.3.4, 5.6.7.8");
        set_mock_field(task_ctx, "request.source_ip", "192.168.1.1");
        let flow = handle.process(ox_plugin_process, task_ctx);
        assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
        let updated_ip = get_mock_field(task_ctx, "request.source_ip").unwrap_or_default();
        assert_eq!(updated_ip, "1.2.3.4");
        unsafe { drop_task_state(task_ctx); }
    }

    // Test 2: Malformed IP value — should not panic
    {
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.header.X-Forwarded-For", "malformed_ip_value");
        let flow = handle.process(ox_plugin_process, task_ctx);
        assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
        unsafe { drop_task_state(task_ctx); }
    }

    // Test 3: No header present — source IP unchanged
    {
        let task_ctx = create_task_state();
        set_mock_field(task_ctx, "request.source_ip", "10.0.0.1");
        let flow = handle.process(ox_plugin_process, task_ctx);
        assert_eq!(flow.code, FLOW_CONTROL_CONTINUE);
        unsafe { drop_task_state(task_ctx); }
    }
}
