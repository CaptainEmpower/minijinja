//! `Environment::set_native_captures`: a body whose whole output is one value
//! answers that value, as Jinja2's native `concat` does.
//!
//! The rows are ansible-core 2.19.13's, which renders with a native
//! environment; each is `type_debug`/value as measured there.
#![cfg(feature = "macros")]
use minijinja::value::{Value, ValueKind};
use minijinja::{context, Environment};

fn native_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_native_captures(true);
    env
}

fn render(env: &Environment, source: &str) -> (String, Option<Value>) {
    env.template_from_str(source)
        .unwrap()
        .render_with_value(context! { n => 5 })
        .unwrap()
}

/// What `x` is after the template, read back through `{{ x }}`.
fn kind_of(source: &str) -> ValueKind {
    render(&native_env(), source)
        .1
        .map_or(ValueKind::String, |v| v.kind())
}

#[test]
fn a_set_block_of_one_value_is_that_value() {
    let env = native_env();
    assert_eq!(
        render(&env, "{% set x %}{{ 5 }}{% endset %}{{ x + 1 }}").1,
        Some(Value::from(6))
    );
    assert_eq!(
        render(&env, "{% set x %}{{ [1, 2] }}{% endset %}{{ x | length }}").1,
        Some(Value::from(2))
    );
    assert_eq!(
        kind_of("{% set x %}{{ none }}{% endset %}{{ x }}"),
        ValueKind::None
    );
}

#[test]
fn a_set_block_of_anything_else_is_its_text() {
    for source in [
        "{% set x %}a{{ 5 }}{% endset %}{{ x }}",
        "{% set x %} {{ 5 }}{% endset %}{{ x }}",
        "{% set x %}{{ '5' }}{% endset %}{{ x }}",
        "{% set x %}{{ 5 }}{{ 6 }}{% endset %}{{ x }}",
        "{% set x %}5{% endset %}{{ x }}",
    ] {
        assert_eq!(kind_of(source), ValueKind::String, "{source}");
    }
    let (text, value) = render(
        &native_env(),
        "{% set x %}{{ 5 }}{% endset %}{{ x }}{{ x }}",
    );
    assert_eq!((text.as_str(), value), ("55", None));
}

#[test]
fn a_macro_body_of_one_value_is_that_value() {
    let env = native_env();
    assert_eq!(
        render(&env, "{% macro m() %}{{ 5 }}{% endmacro %}{{ m() + 1 }}").1,
        Some(Value::from(6))
    );
    assert_eq!(
        kind_of("{% macro m() %}{{ n }}{% endmacro %}{{ m() }}"),
        ValueKind::Number
    );
    assert_eq!(
        kind_of("{% macro m() %}5{% endmacro %}{{ m() }}"),
        ValueKind::String
    );
    assert_eq!(
        kind_of("{% macro m() %}x{{ 5 }}{% endmacro %}{{ m() }}"),
        ValueKind::String
    );
}

#[test]
fn a_caller_body_of_one_value_is_that_value() {
    let env = native_env();
    let source = "{% macro m() %}{{ caller() + 1 }}{% endmacro %}\
                  {% call m() %}{{ 5 }}{% endcall %}";
    assert_eq!(render(&env, source).1, Some(Value::from(6)));
}

#[test]
fn a_filter_block_sees_the_value_and_emits_one() {
    let env = native_env();
    assert_eq!(
        render(&env, "{% filter int %}{{ 5 }}{% endfilter %}").1,
        Some(Value::from(5))
    );
    assert_eq!(
        kind_of("{% set y %}{% filter int %}{{ 5 }}{% endfilter %}{% endset %}{{ y }}"),
        ValueKind::Number
    );
}

/// A sole value at the top level, through any statement that emits nothing.
#[test]
fn the_template_answers_its_own_sole_value() {
    let env = native_env();
    for source in [
        "{{ n }}",
        "{% if true %}{{ n }}{% endif %}",
        "{# c #}{{ n }}",
        "{% set y = 0 %}{{ n + y }}",
    ] {
        assert_eq!(
            render(&env, source),
            ("5".into(), Some(Value::from(5))),
            "{source}"
        );
    }
    assert_eq!(
        render(&env, "{% for i in [1] %}{{ [i, n] }}{% endfor %}").1,
        Some(Value::from(vec![1, 5]))
    );
    for source in ["{{ n }} ", "{{ n }}{{ n }}", "{{ 'n' }}", "5"] {
        assert_eq!(render(&env, source).1, None, "{source}");
    }
}

/// Native captures are opt-in: without them every body is its text.
#[test]
fn off_by_default() {
    let env = Environment::new();
    assert!(!env.native_captures());
    assert_eq!(render(&env, "{{ n }}").1, None);
    assert_eq!(
        render(&env, "{% set x %}{{ 5 }}{% endset %}{{ x ~ 1 }}").0,
        "51"
    );
}

/// A macro defined in a body is still answered as text, never escaping its
/// macro's state.
#[test]
fn a_macro_object_is_not_returned_from_a_macro() {
    let env = native_env();
    let source = "{% macro outer() %}{% macro inner() %}i{% endmacro %}{{ inner }}{% endmacro %}\
                  {{ outer() is string }}";
    assert_eq!(render(&env, source).0, "True");
}

/// An empty capture is `none`, as Jinja2's native `concat` answers `None` for
/// an empty node list. Measured on ansible-core 2.21.5, each is `NoneType`.
#[test]
fn an_empty_capture_is_none() {
    for source in [
        "{% set x %}{% endset %}{{ x }}",
        "{% set x %}{% if false %}a{% endif %}{% endset %}{{ x }}",
        "{% macro m() %}{% endmacro %}{{ m() }}",
        "{% macro m() %}{{ caller() }}{% endmacro %}{% call m() %}{% endcall %}",
    ] {
        assert_eq!(kind_of(source), ValueKind::None, "{source}");
    }
    // A template whose own output is empty answers `none` too.
    assert_eq!(
        render(&native_env(), "{% if false %}x{% endif %}").1,
        Some(Value::from(()))
    );
    // Whitespace is text, not nothing.
    assert_eq!(
        kind_of("{% set x %} {% endset %}{{ x }}"),
        ValueKind::String
    );
}
