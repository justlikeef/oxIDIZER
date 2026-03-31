#[cfg(test)]
mod tests {
    use crate::{ox_plugin_init, ox_plugin_process, ox_plugin_destroy};
    use ox_webservice_test_utils::{
        create_mock_api, create_task_state, drop_task_state, get_mock_field, set_mock_field,
    };
    use ox_workflow_abi::{FLOW_CONTROL_CONTINUE, OX_WORKFLOW_ABI_VERSION};
    use std::ffi::CString;

    #[test]
    fn test_ping_basic() {
        let api = create_mock_api();
        let task_ctx = create_task_state();

        let c_config = CString::new("{}").unwrap();
        let config_ctx = unsafe {
            ox_plugin_init(c_config.as_ptr(), &api as *const _, OX_WORKFLOW_ABI_VERSION)
        };
        assert!(!config_ctx.is_null(), "ox_plugin_init returned null");

        set_mock_field(task_ctx, "request.header.accept", "application/json");

        let fc = unsafe { ox_plugin_process(config_ctx, task_ctx) };
        assert_eq!(fc.code, FLOW_CONTROL_CONTINUE);
        assert_eq!(get_mock_field(task_ctx, "response.status").as_deref(), Some("200"));
        assert!(
            get_mock_field(task_ctx, "response.body").unwrap_or_default().contains("pong"),
            "response body should contain pong"
        );

        unsafe { ox_plugin_destroy(config_ctx); }
        unsafe { drop_task_state(task_ctx); }
    }

    #[test]
    fn test_ping_html_format() {
        let api = create_mock_api();
        let task_ctx = create_task_state();

        let c_config = CString::new("{}").unwrap();
        let config_ctx = unsafe {
            ox_plugin_init(c_config.as_ptr(), &api as *const _, OX_WORKFLOW_ABI_VERSION)
        };
        assert!(!config_ctx.is_null());

        // No Accept: application/json → HTML format
        set_mock_field(task_ctx, "request.header.accept", "text/html");

        let fc = unsafe { ox_plugin_process(config_ctx, task_ctx) };
        assert_eq!(fc.code, FLOW_CONTROL_CONTINUE);
        assert_eq!(get_mock_field(task_ctx, "response.status").as_deref(), Some("200"));
        assert!(
            get_mock_field(task_ctx, "response.body").unwrap_or_default().contains("pong"),
            "response body should contain pong"
        );
        assert_eq!(
            get_mock_field(task_ctx, "response.header.Content-Type").as_deref(),
            Some("text/html")
        );

        unsafe { ox_plugin_destroy(config_ctx); }
        unsafe { drop_task_state(task_ctx); }
    }
}
