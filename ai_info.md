
PROJECT_ROOT: /mnt/c/Users/justl/source/repos/oxIDIZER
FOCUSED_CRATES: ox_webservice ox_webservice_errorhandler_jinja2 ox_webservice_api ox_content ox_server_info
USER_CONSTRAINTS:
  - NO_UNAPPROVED_CHANGES
  - USE_ABSOLUTE_PATHS
ARCHITECTURE_OVERVIEW:
  - Main executable: ox_webservice
  - Asynchronous, multithreaded runtime via tokio.
  - Web server built with axum framework.
  - Core logic extended via dynamically loaded modules (.so/.dll files).
  - Shared API crate (ox_webservice_api) for communication between executable and modules.
  - Functional tests are a key part of the project structure.
KEY_CRATES_AND_FRAMEWORKS:
  - WEB_FRAMEWORK: axum (with tower/tower-http ecosystem for services/middleware).
  - ASYNC_RUNTIME: tokio.
  - LOGGING: log4rs (configured via YAML file).
  - SERIALIZATION: serde (for JSON, YAML, etc.).
  - CLI_PARSING: clap.
  - DYNAMIC_LOADING: libloading (C-style dynamic loading).
  - TEMPLATING: tera.
RECENT_CHANGES:
  - FILE: start_server.sh
    DESCRIPTION: Modified to accept optional config file path.
  - FILE: ox_webservice_errorhandler_jinja2/functional_tests/000001-ConfigNotFound/test.sh
    DESCRIPTION: Created/modified. Verifies server PID, checks for log panics.
  - CRATE: ox_webservice
    DESCRIPTION: Switched to log4rs for logging.
    SUB_CHANGES:
      - FILE: Cargo.toml
        DESCRIPTION: Added log4rs dependency.
      - FILE: src/main.rs
        DESCRIPTION:
          - ServerConfig struct updated (log4rs_config added, old log fields removed).
          - Initial env_logger removed.
          - Custom panic hook removed.
          - Output redirection logic removed.
          - log4rs initialized after main config load.
          - Panic on invalid ox_webservice.yaml fixed.
          - "Logger already initialized" error fixed.
