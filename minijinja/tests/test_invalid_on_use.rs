//! `Environment::set_invalid_raises_on_use` and `Environment::set_call_guard`.
#![cfg(feature = "builtins")]

use minijinja::value::{Value, ValueKind};
use minijinja::{CallKind, Environment, Error, ErrorKind};

fn poison() -> Value {
    Value::from(Error::new(ErrorKind::InvalidOperation, "poisoned"))
}

fn render(env: &Environment, source: &str) -> Result<String, Error> {
    env.render_str(source, minijinja::context! { secret => poison(), n => 2 })
}

fn failed(result: Result<String, Error>) -> bool {
    result.is_err_and(|err| err.to_string().contains("poisoned"))
}

fn on_use() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_invalid_raises_on_use(true);
    env
}

#[test]
fn by_default_an_invalid_value_raises_when_loaded() {
    let env = Environment::new();
    assert!(failed(render(&env, "{{ [secret] | length }}")));
    // A branch that is never evaluated never loads it, in either mode.
    assert_eq!(render(&env, "{{ true or secret }}").unwrap(), "True");
}

#[test]
fn on_use_a_container_can_hold_one() {
    let env = on_use();
    assert_eq!(render(&env, "{{ [secret] | length }}").unwrap(), "1");
    assert_eq!(render(&env, "{{ [secret, 1] | length }}").unwrap(), "2");
    assert_eq!(render(&env, "{{ {'a': secret} | length }}").unwrap(), "1");
    assert_eq!(render(&env, "{% set x = secret %}ok").unwrap(), "ok");
    assert_eq!(render(&env, "{{ true or secret }}").unwrap(), "True");
}

#[test]
fn on_use_reading_one_out_of_a_container_holds_it_too() {
    let env = on_use();
    assert_eq!(render(&env, "{{ [[secret][0]] | length }}").unwrap(), "1");
    assert_eq!(
        render(&env, "{{ [{'a': secret}.a] | length }}").unwrap(),
        "1"
    );
}

#[test]
fn on_use_using_one_raises() {
    let env = on_use();
    for source in [
        "{{ secret }}",
        "{{ secret == 1 }}",
        "{{ secret != 1 }}",
        "{{ secret < 1 }}",
        "{{ secret + 1 }}",
        "{{ -secret }}",
        "{{ not secret }}",
        "{{ secret ~ 'a' }}",
        "{{ secret in [1] }}",
        "{{ 1 in secret }}",
        "{{ 1 in [secret] }}",
        "{{ 1 not in [2, secret] }}",
        "{{ 'y' if secret else 'n' }}",
        "{{ secret and 1 }}",
        "{{ secret or 1 }}",
        "{% for x in secret %}{% endfor %}",
        "{{ secret.attr }}",
        "{{ secret['key'] }}",
        "{{ secret[1:] }}",
        "{{ secret.upper() }}",
        "{{ secret() }}",
        "{% set a, b = secret %}",
        "{{ [secret] | first }}",
        "{{ 1 < secret < 3 }}",
    ] {
        assert!(failed(render(&env, source)), "{source} should raise");
    }
}

/// A containment check compares in order, so a match before the invalid item
/// ends it, and a mapping is checked by its keys.
#[test]
fn on_use_a_containment_check_uses_only_what_it_compares() {
    let env = on_use();
    assert_eq!(render(&env, "{{ 1 in [1, secret] }}").unwrap(), "True");
    assert_eq!(render(&env, "{{ 'a' in {'a': secret} }}").unwrap(), "True");
}

#[test]
fn a_guard_can_hand_back_a_value_for_a_filter_or_a_test() {
    let mut env = on_use();
    env.add_filter("kind", |_: Value| "plain");
    env.set_call_guard(|_, kind, name, args| {
        let held = args
            .first()
            .is_some_and(|arg| arg.kind() == ValueKind::Invalid);
        Ok(match (kind, name) {
            (CallKind::Filter, "kind") if held => Some(Value::from("sealed")),
            (CallKind::Filter | CallKind::Test, _) if held => args.first().cloned(),
            _ => None,
        })
    });
    assert_eq!(render(&env, "{{ secret | kind }}").unwrap(), "sealed");
    // The guard's value carries on: the test yields the invalid value itself,
    // and the filter after it names it.
    assert_eq!(
        render(&env, "{{ (secret is defined) | kind }}").unwrap(),
        "sealed"
    );
    assert!(failed(render(&env, "{{ secret | upper }}")));
    assert!(failed(render(&env, "{{ 'y' if secret is defined }}")));
    assert_eq!(render(&env, "{{ n | upper }}").unwrap(), "2");
}

#[test]
fn a_guard_sees_filters_applied_through_the_state() {
    let mut env = on_use();
    env.set_call_guard(|_, kind, name, args| {
        if kind == CallKind::Filter
            && name == "upper"
            && args.iter().any(|arg| arg.kind() == ValueKind::Invalid)
        {
            return Err(Error::new(ErrorKind::InvalidOperation, "guarded"));
        }
        Ok(None)
    });
    let result = render(&env, "{{ [secret] | map('upper') | list }}");
    assert!(result.is_err_and(|err| err.to_string().contains("guarded")));
}

#[test]
fn a_guard_can_refuse_a_function_or_a_method() {
    let mut env = on_use();
    env.set_call_guard(|_, kind, name, _| {
        Ok(match (kind, name) {
            (CallKind::Function, "range") => Some(Value::from("no range")),
            (CallKind::Method, "items") => Some(Value::from("no items")),
            _ => None,
        })
    });
    assert_eq!(render(&env, "{{ range(3) }}").unwrap(), "no range");
    assert_eq!(render(&env, "{{ {'a': 1}.items() }}").unwrap(), "no items");
}
