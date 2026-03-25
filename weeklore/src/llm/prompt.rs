pub fn fill_prompt(template: &str, vars: &[(&str, &str)]) -> String {
    let mut result = template.to_string();

    for (k, v) in vars {
        result = result.replace(&format!("{{{}}}", k), v);
    }

    result
}
