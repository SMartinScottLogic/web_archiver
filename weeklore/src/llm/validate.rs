pub fn validate_bullets(output: &str, min: usize, max: usize) -> bool {
    let count = output
        .lines()
        .filter(|l| l.trim().starts_with(['-', '*']))
        .count();

    count >= min && count <= max
}

pub fn extract_json(output: &str) -> Option<String> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    Some(output[start..=end].to_string())
}

pub fn validate_json<T: serde::de::DeserializeOwned>(output: &str) -> Option<T> {
    let json_str = extract_json(output)?;
    serde_json::from_str(&json_str).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn validate_bullets_empty() {
        assert!(!validate_bullets("", 1, 3));
        assert!(validate_bullets("", 0, 3));
    }

    #[test]
    fn validate_bullets_fits() {
        let input = r#"Non bullet lead-in
            - bullet 1
            - bullet 2
            * bullet 3
        "#;
        assert!(validate_bullets(input, 3, 5));
    }

    #[test]
    fn validate_bullets_too_few() {
        let input = r#"Non bullet lead-in
            - bullet 1
            - bullet 2
            * bullet 3
        "#;
        assert!(!validate_bullets(input, 5, 7));
    }

    #[test]
    fn validate_bullets_too_many() {
        let input = r#"Non bullet lead-in
            - bullet 1
            - bullet 2
            * bullet 3
        "#;
        assert!(!validate_bullets(input, 0, 2));
    }

    #[test]
    fn extract_json_plain_text() {
        let input = r#"This is some text.

        It is rambling and fails to get to any point, and crucially:
        IT CONTAINS NO JSON!
        "#;
        assert!(extract_json(input).is_none());
    }

    #[test]
    fn extract_json_left_brace() {
        let input = r#"This is some text.

        It is rambling and fails to get to any point, and crucially:
        IT CONTAINS ONLY OPEN BRACE: {
        "#;
        assert!(extract_json(input).is_none());
    }

    #[test]
    fn extract_json_right_brace() {
        let input = r#"This is some text.

        It is rambling and fails to get to any point, and crucially:
        IT CONTAINS ONLY CLOSE BRACE: }
        "#;
        assert!(extract_json(input).is_none());
    }

    #[test]
    fn extract_json_whatever_between() {
        let input = r#"This is some text.

        It is rambling and fails to get to any point, and crucially:
        IT CONTAINS THE FOLLOWING BLOB: {{{[]}}
        "#;
        assert_eq!(Some("{{{[]}}"), extract_json(input).as_deref());
    }

    #[test]
    fn validate_json_whatever_between() {
        let input = r#"This is some text.

        It is rambling and fails to get to any point, and crucially:
        IT CONTAINS THE FOLLOWING BLOB: {{{[]}}
        "#;
        assert!(validate_json::<Value>(input).is_none());
    }

    #[test]
    fn validate_json_valid_between() {
        let input = r#"This is some text.

        It is rambling and fails to get to any point, and crucially:
        IT CONTAINS THE FOLLOWING BLOB: {"test":true, "null": null, "num": 1, "obj": {}, "array": [1,2,3]}
        "#;
        let result = validate_json::<Value>(input);
        assert!(result.is_some());
        let result = result.unwrap();
        let result = result.as_object().unwrap();
        assert!(result.get("test").unwrap().as_bool().unwrap());
        assert!(result.get("null").unwrap().as_null().is_some());
        assert_eq!(1, result.get("num").unwrap().as_i64().unwrap());
        assert!(result.get("obj").unwrap().as_object().unwrap().is_empty());
        assert_eq!(
            [Value::from(1), Value::from(2), Value::from(3)],
            result.get("array").unwrap().as_array().unwrap().as_slice()
        );
    }
}
