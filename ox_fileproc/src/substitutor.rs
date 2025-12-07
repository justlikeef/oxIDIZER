use std::collections::HashMap;
use regex::Regex;
use std::sync::OnceLock;

static VAR_REGEX: OnceLock<Regex> = OnceLock::new();

fn get_regex() -> &'static Regex {
    VAR_REGEX.get_or_init(|| Regex::new(r"\$\{([a-zA-Z0-9_]+)\}").unwrap())
}

/// Performs variable substitution on a string using the provided context.
/// Replaces ${VAR_NAME} with the value from the context.
/// If a variable is not found in the context, it is left as is.
pub fn substitute(input: &str, context: &HashMap<String, String>) -> String {
    let re = get_regex();
    re.replace_all(input, |caps: &regex::Captures| {
        let var_name = &caps[1];
        context.get(var_name).cloned().unwrap_or_else(|| caps[0].to_string())
    }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitution() {
        let mut context = HashMap::new();
        context.insert("NAME".to_string(), "World".to_string());
        context.insert("VAR_1".to_string(), "Value1".to_string());

        assert_eq!(substitute("Hello ${NAME}", &context), "Hello World");
        assert_eq!(substitute("No var here", &context), "No var here");
        assert_eq!(substitute("Missing ${MISSING}", &context), "Missing ${MISSING}");
        assert_eq!(substitute("${VAR_1} and ${NAME}", &context), "Value1 and World");
    }
}
