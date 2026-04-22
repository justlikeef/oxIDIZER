#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Current ABI version for the workflow engine and plugins
 */
#define OX_WORKFLOW_ABI_VERSION 3

/**
 * Minimum ABI version this crate supports
 */
#define OX_WORKFLOW_ABI_MIN_VERSION 3

/**
 * Feature flags for plugin capabilities
 */
#define FEATURE_NONE 0

#define FEATURE_BINARY_DATA (1 << 0)

#define FEATURE_METADATA (1 << 1)

#define FEATURE_FLAGS (1 << 2)

#define FEATURE_FLOW_INSERT (1 << 3)

#define FEATURE_TASK_PAUSE (1 << 4)

#define FEATURE_ASYNC (1 << 5)

#define FEATURE_WASM (1 << 6)

/**
 * Flow control code: Continue to the next plugin or stage.
 */
#define FLOW_CONTROL_CONTINUE 0

/**
 * Flow control code: End the flow successfully.
 */
#define FLOW_CONTROL_END 1

/**
 * Flow control code: Trigger error lifecycle.
 */
#define FLOW_CONTROL_ERROR 2

/**
 * Flow control code: Branch to a specific stage named in `payload`.
 */
#define FLOW_CONTROL_JUMP 3

/**
 * Flow control code: Skip to a plugin named in `payload` within the current stage.
 */
#define FLOW_CONTROL_SKIP 4

/**
 * Flow control code: Pause task. `payload` can specify a timer or signal key.
 */
#define FLOW_CONTROL_SUSPEND 5

#define FLOW_CONTROL_YIELD 6

/**
 * Flow control code: Stream a file from the path in `payload` (a c_char path string).
 */
#define FLOW_CONTROL_STREAM_FILE 7

/**
 * Flag scope: Cleared at each stage boundary.
 */
#define FLAG_SCOPE_STAGE 0

/**
 * Flag scope: Persists with task state across stages.
 */
#define FLAG_SCOPE_TASK 1

/**
 * Log level: error — unrecoverable condition.
 */
#define OX_LOG_ERROR 1

/**
 * Log level: warning — recoverable but noteworthy.
 */
#define OX_LOG_WARN 2

/**
 * Log level: informational.
 */
#define OX_LOG_INFO 3

/**
 * Log level: debug — verbose diagnostic.
 */
#define OX_LOG_DEBUG 4

/**
 * Log level: trace — very verbose, inner-loop detail.
 */
#define OX_LOG_TRACE 5

#define PLUGIN_ABI_VERSION 3

/**
 * Plugin capabilities structure returned during version negotiation.
 * This allows the host to understand what features a plugin supports.
 */
typedef struct PluginCapabilities {
  /**
   * Minimum ABI version the plugin supports
   */
  uint32_t min_abi_version;
  /**
   * Maximum ABI version the plugin supports
   */
  uint32_t max_abi_version;
  /**
   * Bitfield of supported features (use FEATURE_* constants)
   */
  uint64_t features;
  /**
   * Plugin name (null-terminated C string, max 64 chars including null)
   */
  char name[64];
  /**
   * Plugin version (null-terminated C string, max 32 chars including null)
   */
  char version[32];
} PluginCapabilities;

/**
 * Structure returned by plugins to dictate the engine's next action.
 */
typedef struct FlowControl {
  uint8_t code;
  const char *payload;
} FlowControl;

/**
 * Host-provided API function table.
 * Plugins use these functions to read/write state and perform task operations.
 * All strings are null-terminated C strings.
 */
typedef struct CoreHostApi {
  const char *(*get_field)(void *task_ctx, const char *key);
  void (*set_field)(void *task_ctx, const char *key, const char *value);
  /**
   * Read a binary (Bytes) field. Returns null pointer and sets len_out=0 if not found.
   * The returned pointer is valid until the next API call on this task.
   */
  const uint8_t *(*get_field_bytes)(void *task_ctx, const char *key, uintptr_t *len_out);
  /**
   * Write a binary (Bytes) field. Copies `len` bytes from `value`.
   */
  void (*set_field_bytes)(void *task_ctx, const char *key, const uint8_t *value, uintptr_t len);
  const char *(*get_metadata)(void *task_ctx, const char *key);
  bool (*insert_into_flow)(void *task_ctx, const char *flow_name);
  void (*pause_task)(void *task_ctx, const char *signal_key);
  void (*log)(void *task_ctx, uint8_t level, const char *message);
  void (*set_flag)(void *task_ctx, const char *flag, uint8_t scope);
  void (*set_flags)(void *task_ctx, const char *const *flags, uint8_t scope);
  bool (*has_flag)(void *task_ctx, const char *flag, uint8_t scope);
  void (*clear_flag)(void *task_ctx, const char *flag, uint8_t scope);
  /**
   * Get all keys in task context. Returns comma-separated list, empty if none.
   */
  const char *(*get_keys)(void *task_ctx);
  /**
   * Remove a key from task context. Returns 1 if removed, 0 if not found.
   */
  bool (*unset_field)(void *task_ctx, const char *key);
  /**
   * Check if key exists. Returns 1 if exists, 0 if not.
   */
  bool (*has_field)(void *task_ctx, const char *key);
} CoreHostApi;

/**
 * Type representing the plugin initialization function
 */
typedef void *(*OxPluginInitFn)(const char *plugin_config_ctx,
                                const struct CoreHostApi *api,
                                uint32_t abi_version);

/**
 * Type representing the plugin process function
 */
typedef struct FlowControl (*OxPluginProcessFn)(void *plugin_config_ctx, void *task_ctx);

/**
 * Type representing the plugin error callback
 */
typedef void (*OxPluginErrorFn)(void *plugin_config_ctx, void *task_ctx);

/**
 * Type representing the plugin teardown/destroy function
 */
typedef void (*OxPluginDestroyFn)(void *plugin_config_ctx);

/**
 * Frees capabilities returned by negotiate function.
 * Should be called by host after obtaining capabilities.
 */
void free_plugin_caps(struct PluginCapabilities *caps);

void _ox_workflow_dummy_export(struct FlowControl _fc,
                               struct CoreHostApi _api,
                               OxPluginInitFn _init,
                               OxPluginProcessFn _proc,
                               OxPluginErrorFn _err,
                               OxPluginDestroyFn _destroy);
