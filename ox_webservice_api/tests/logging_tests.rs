use ox_webservice_api::{LogLevel, webservice_log};
use std::ffi::CString;
use std::sync::Mutex;
use log::{self, Log, Metadata, Record, set_logger, set_max_level, LevelFilter};

// A static, mutable vector to store log messages received by the mock callback.
// Mutex is used to allow safe concurrent access in tests.
static LOG_MESSAGES: Mutex<Vec<(LogLevel, String, String)>> = Mutex::new(Vec::new());

struct MockLogger;

impl Log for MockLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level = match record.level() {
                log::Level::Error => LogLevel::Error,
                log::Level::Warn => LogLevel::Warn,
                log::Level::Info => LogLevel::Info,
                log::Level::Debug => LogLevel::Debug,
                log::Level::Trace => LogLevel::Trace,
            };
            LOG_MESSAGES.lock().unwrap().push((
                level,
                record.target().to_string(),
                format!("{}", record.args()),
            ));
        }
    }

    fn flush(&self) {}
}

static LOGGER: MockLogger = MockLogger;

#[test]
fn test_logging_functions() {
    set_logger(&LOGGER).unwrap();
    set_max_level(LevelFilter::Trace);
    // Test Error logging
    let module_error = CString::new("test_module_error").expect("CString::new failed");
    let message_error = CString::new("This is an error message").expect("CString::new failed");
    unsafe {
        webservice_log(LogLevel::Error, module_error.as_ptr(), message_error.as_ptr());
    }

    // Test Warn logging
    let module_warn = CString::new("test_module_warn").expect("CString::new failed");
    let message_warn = CString::new("This is a warning message").expect("CString::new failed");
    unsafe {
        webservice_log(LogLevel::Warn, module_warn.as_ptr(), message_warn.as_ptr());
    }

    // Test Info logging
    let module_info = CString::new("test_module_info").expect("CString::new failed");
    let message_info = CString::new("This is an info message").expect("CString::new failed");
    unsafe {
        webservice_log(LogLevel::Info, module_info.as_ptr(), message_info.as_ptr());
    }

    // Test Debug logging
    let module_debug = CString::new("test_module_debug").expect("CString::new failed");
    let message_debug = CString::new("This is a debug message").expect("CString::new failed");
    unsafe {
        webservice_log(LogLevel::Debug, module_debug.as_ptr(), message_debug.as_ptr());
    }

    // Test Trace logging
    let module_trace = CString::new("test_module_trace").expect("CString::new failed");
    let message_trace = CString::new("This is a trace message").expect("CString::new failed");
    unsafe {
        webservice_log(LogLevel::Trace, module_trace.as_ptr(), message_trace.as_ptr());
    }

    let messages = LOG_MESSAGES.lock().unwrap();
    assert_eq!(messages.len(), 5);

    assert_eq!(messages[0].0, LogLevel::Error);
    assert_eq!(messages[0].1, "test_module_error");
    assert_eq!(messages[0].2, "This is an error message");

    assert_eq!(messages[1].0, LogLevel::Warn);
    assert_eq!(messages[1].1, "test_module_warn");
    assert_eq!(messages[1].2, "This is a warning message");

    assert_eq!(messages[2].0, LogLevel::Info);
    assert_eq!(messages[2].1, "test_module_info");
    assert_eq!(messages[2].2, "This is an info message");

    assert_eq!(messages[3].0, LogLevel::Debug);
    assert_eq!(messages[3].1, "test_module_debug");
    assert_eq!(messages[3].2, "This is a debug message");

    assert_eq!(messages[4].0, LogLevel::Trace);
    assert_eq!(messages[4].1, "test_module_trace");
    assert_eq!(messages[4].2, "This is a trace message");
}
