pub fn fill_prompt(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();

    for (k, v) in vars {
        result = result.replace(&format!("{{{}}}", k), v);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_single_variable() {
        let template = "Hello {name}!";
        let vars = &[("name", "Alice")];

        let result = fill_prompt(template, vars);
        assert_eq!(result, "Hello Alice!");
    }

    #[test]
    fn replaces_multiple_variables() {
        let template = "Hello {name}, welcome to {place}.";
        let vars = &[("name", "Alice"), ("place", "Wonderland")];

        let result = fill_prompt(template, vars);
        assert_eq!(result, "Hello Alice, welcome to Wonderland.");
    }

    #[test]
    fn leaves_unknown_variables_unchanged() {
        let template = "Hello {name}, age {age}";
        let vars = &[("name", "Alice")];

        let result = fill_prompt(template, vars);
        assert_eq!(result, "Hello Alice, age {age}");
    }

    #[test]
    fn replaces_multiple_occurrences_of_same_variable() {
        let template = "{word} {word} {word}";
        let vars = &[("word", "echo")];

        let result = fill_prompt(template, vars);
        assert_eq!(result, "echo echo echo");
    }

    #[test]
    fn empty_vars_returns_original_string() {
        let template = "Hello {name}";
        let vars: &[(&str, &str)] = &[];

        let result = fill_prompt(template, vars);
        assert_eq!(result, "Hello {name}");
    }

    #[test]
    fn variables_do_not_interfere_with_similar_names() {
        let template = "{name} vs {name_full}";
        let vars = &[("name", "Alice"), ("name_full", "Alice Smith")];

        let result = fill_prompt(template, vars);
        assert_eq!(result, "Alice vs Alice Smith");
    }

    #[test]
    fn order_of_vars_does_not_affect_result() {
        let template = "{a} {b}";
        let vars1 = &[("a", "1"), ("b", "2")];
        let vars2 = &[("b", "2"), ("a", "1")];

        let r1 = fill_prompt(template, vars1);
        let r2 = fill_prompt(template, vars2);

        assert_eq!(r1, r2);
        assert_eq!(r1, "1 2");
    }
}
