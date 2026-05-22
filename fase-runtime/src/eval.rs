use std::collections::BTreeMap;

use cel::{Program, Value};

use super::*;

#[derive(Debug, Clone, Default)]
pub struct Evaluator {
    bindings: BTreeMap<String, String>,
}

impl Evaluator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with<I, K, V>(bindings: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            bindings: bindings
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }

    pub fn bind(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.bindings.insert(key.into(), value.into());
        self
    }

    pub fn eval(&self, expression: &str) -> Result<String> {
        let program = Program::compile(expression).map_err(|error| {
            invalid_input(format!(
                "failed to compile CEL expression {expression:?}: {error}"
            ))
        })?;

        let mut context = cel::Context::default();
        for (key, value) in &self.bindings {
            if is_identifier(key) {
                context.add_variable(key, value.as_str())?;
            }
        }
        context.add_variable("vars", self.bindings.clone())?;

        value_string(program.execute(&context).map_err(|error| {
            invalid_input(format!(
                "failed to evaluate CEL expression {expression:?}: {error}"
            ))
        })?)
    }

    pub fn render(&self, template: &str) -> Result<String> {
        let mut output = String::new();
        let mut rest = template;

        while let Some(start) = rest.find("${") {
            output.push_str(&rest[..start]);
            let expression = &rest[start + 2..];
            let Some(end) = expression.find('}') else {
                return Err(invalid_input(format!("unclosed expression in {template:?}")).into());
            };

            let expression = expression[..end].trim();
            if expression.is_empty() {
                return Err(invalid_input(format!("empty expression in {template:?}")).into());
            }
            output.push_str(&self.eval(expression)?);
            rest = &rest[start + 2 + end + 1..];
        }

        output.push_str(rest);
        Ok(output)
    }
}

fn value_string(value: Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.as_ref().clone()),
        Value::Int(value) => Ok(value.to_string()),
        Value::UInt(value) => Ok(value.to_string()),
        Value::Float(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Bytes(value) => Ok(String::from_utf8_lossy(value.as_slice()).into_owned()),
        Value::Null => Ok(String::new()),
        value => Err(invalid_input(format!(
            "CEL expression returned {}, expected scalar value",
            value.type_of()
        ))
        .into()),
    }
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && chars.all(|ch| matches!(ch, '_' | 'a'..='z' | 'A'..='Z' | '0'..='9'))
}

fn invalid_input(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn render() {
        let evaluator = Evaluator::with([("name", "source"), ("extension", "tar.gz")]);
        assert_eq!(
            evaluator
                .render("https://example.com/${name + \".\" + extension}")
                .unwrap(),
            "https://example.com/source.tar.gz"
        );
    }

    #[test]
    fn render_vars_map() {
        let evaluator = Evaluator::with([("source-url", "https://example.com/source.tar.gz")]);
        assert_eq!(
            evaluator.render("${vars[\"source-url\"]}").unwrap(),
            "https://example.com/source.tar.gz"
        );
    }

    #[test]
    fn render_literal() {
        assert_eq!(
            Evaluator::new().render("source.tar.gz").unwrap(),
            "source.tar.gz"
        );
    }
}
