use std::collections::HashMap;
use regex::Regex;
use std::sync::OnceLock;

static VAR_REGEX: OnceLock<Regex> = OnceLock::new();

fn get_regex() -> &'static Regex {
    // Match 0 or more backslashes, then ${{VAR_NAME}}
    VAR_REGEX.get_or_init(|| Regex::new(r"(\\*)?\$\{\{([a-zA-Z0-9_]+)\}\}").unwrap())
}

/// Performs variable substitution on a string using the provided context.
/// Replaces ${{VAR_NAME}} with the value from the context.
/// Supports escaping with backslashes:
/// - Odd backslashes (`\`, `\\\`) escape the token: `\${{VAR}}` -> `${{VAR}}`
/// - Even backslashes (`\\`) escape the backslash: `\\${{VAR}}` -> `\` + Value
pub fn substitute(input: &str, context: &HashMap<String, String>, allow_env: bool) -> String {
    let re = get_regex();
    re.replace_all(input, |caps: &regex::Captures| {
        let text_slash = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let slash_count = text_slash.len();
        
        let var_name = &caps[2];
        // let _original_token = &caps[0]; // Unused
        
        if slash_count % 2 == 1 {
            // Odd: Escaped. Reduce backslashes by half (integer division) and keep literal token.
            // e.g. 1 slash -> 0 slashes + ${{...}}
            // e.g. 3 slashes -> 1 slash + ${{...}}
            let kept_slashes = "\\".repeat((slash_count - 1) / 2);
            format!("{}${{{{{}}}}}", kept_slashes, var_name)
        } else {
            // Even: Substitute. Reduce backslashes by half.
            // e.g. 0 slashes -> 0 slashes + Value
            // e.g. 2 slashes -> 1 slash + Value
            let kept_slashes = "\\".repeat(slash_count / 2);
            
            let val = context.get(var_name).cloned()
                .or_else(|| if allow_env { std::env::var(var_name).ok() } else { None })
                .unwrap_or_else(|| format!("${{{{{}}}}}", var_name)); // Fallback to literal if missing (but resolve_vars usually errors)
                
            format!("{}{}", kept_slashes, val)
        }
    }).to_string()
}

/// Checks if the input string contains any unresolved variable placeholders.
/// Ignores escaped placeholders (odd number of preceding backslashes).
pub fn has_unresolved_tokens(input: &str) -> bool {
    let re = get_regex();
    for cap in re.captures_iter(input) {
        let text_slash = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let slash_count = text_slash.len();
        
        // If backslash count is even, then it was NOT escaped (it was a substitution target)
        // If it's still present in output, it's unresolved.
        // If odd, it was escaped, so it's a literal string, NOT unresolved.
        if slash_count % 2 == 0 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitution() {
        let mut context = HashMap::new();
        context.insert("NAME".to_string(), "World".to_string());
        context.insert("VAR_1".to_string(), "Value1".to_string());

        // Strict syntax check
        assert_eq!(substitute("Hello ${{NAME}}", &context, false), "Hello World");
        // Malformed should NOT match
        assert_eq!(substitute("Hello ${NAME}", &context, false), "Hello ${NAME}");
        assert_eq!(substitute("Hello ${NAME}}", &context, false), "Hello ${NAME}}");
        assert_eq!(substitute("Hello ${{NAME}", &context, false), "Hello ${{NAME}");
        
        assert_eq!(substitute("No var here", &context, false), "No var here");
        assert_eq!(substitute("Missing ${{MISSING}}", &context, false), "Missing ${{MISSING}}");
        assert_eq!(substitute("${{VAR_1}} and ${{NAME}}", &context, false), "Value1 and World");
        
        // Test Environment Variable Fallback
        // SAFETY: Safe for test environment where no other threads are accessing env vars concurrently
        unsafe {
            std::env::set_var("TEST_ENV_VAR", "EnvValue");
        }
        
        // With allow_env = true
        assert_eq!(substitute("From Env: ${{TEST_ENV_VAR}}", &context, true), "From Env: EnvValue");
        
        // With allow_env = false
        assert_eq!(substitute("From Env: ${{TEST_ENV_VAR}}", &context, false), "From Env: ${{TEST_ENV_VAR}}");
        
        // Context override checks (always takes precedence)
        context.insert("TEST_ENV_VAR".to_string(), "ContextValue".to_string());
        assert_eq!(substitute("From Env: ${{TEST_ENV_VAR}}", &context, true), "From Env: ContextValue");
        
        // Escape mechanism tests
        // 1. One backslash (Escaped): \${{NAME}} -> ${{NAME}}
        assert_eq!(substitute(r"Escaped: \${{NAME}}", &context, false), r"Escaped: ${{NAME}}");
        // 2. Two backslashes (Literal Backslash + Sub): \\${{NAME}} -> \World
        assert_eq!(substitute(r"BS + Sub: \\${{NAME}}", &context, false), r"BS + Sub: \World");
        // 3. Three backslashes (Literal BS + Escaped): \\\${{NAME}} -> \${{NAME}}
        assert_eq!(substitute(r"BS + Esc: \\\${{NAME}}", &context, false), r"BS + Esc: \${{NAME}}");
        
        // Unresolved check tests
        assert!(has_unresolved_tokens("Some ${{VAR}}")); // 0 slashes = active
        assert!(!has_unresolved_tokens(r"Some \${{VAR}}")); // 1 slash = escaped
        assert!(has_unresolved_tokens(r"Some \\${{VAR}}")); // 2 slashes = active (escaped backslash)
        assert!(!has_unresolved_tokens(r"Some \\\${{VAR}}")); // 3 slashes = escaped
    }
}
