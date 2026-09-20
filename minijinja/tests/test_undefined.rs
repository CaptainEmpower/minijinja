#![cfg(feature = "builtins")]
use std::collections::HashMap;

use minijinja::{context, render, Environment, ErrorKind, State, UndefinedBehavior};

use similar_asserts::assert_eq;

#[test]
fn test_lenient_undefined() {
    let mut env = Environment::new();
    env.add_filter("test", |state: &State, value: String| -> String {
        assert_eq!(state.undefined_behavior(), UndefinedBehavior::Lenient);
        assert_eq!(value, "");
        value
    });

    assert_eq!(env.undefined_behavior(), UndefinedBehavior::Lenient);
    assert_eq!(render!(in env, "<{{ true.missing_attribute }}>"), "<>");
    assert_eq!(
        env.render_str("{{ undefined.missing_attribute }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        render!(in env, "<{% for x in undefined %}...{% endfor %}>"),
        "<>"
    );
    assert_eq!(render!(in env, "{{ 'foo' is in(undefined) }}"), "False");
    assert_eq!(render!(in env, "<{{ undefined }}>"), "<>");
    assert_eq!(render!(in env, "{{ not undefined }}"), "True");
    assert_eq!(render!(in env, "{{ undefined is undefined }}"), "True");
    assert_eq!(
        render!(in env, "{{ x.foo is undefined }}", x => HashMap::<String, String>::new()),
        "True"
    );
    assert_eq!(render!(in env, "{{ undefined|list }}"), "[]");
    assert_eq!(render!(in env, "<{{ undefined|test }}>"), "<>");
    assert_eq!(render!(in env, "{{ 42 in undefined }}"), "False");
}

/// `StrictChainable` is `Strict` in every respect but one: a lookup *through*
/// an undefined yields an undefined instead of failing.
///
/// Each assertion below is paired with what the two neighbouring modes answer,
/// because the point of the variant is that neither of them answers all of it:
/// `Chainable` passes the first group and fails the second, `Strict` the
/// reverse.
#[test]
fn test_strict_chainable_undefined() {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::StrictChainable);

    // Chains resolve, as under `Chainable` and unlike `Strict`.
    assert_eq!(
        render!(in env, "{{ undefined.missing_attribute|default('D') }}"),
        "D"
    );
    assert_eq!(render!(in env, "{{ undefined.a.b.c|default('D') }}"), "D");
    assert_eq!(render!(in env, "{{ undefined['key']|default('D') }}"), "D");
    assert_eq!(
        render!(in env, "{{ undefined.missing is undefined }}"),
        "True"
    );
    assert_eq!(
        render!(in env, "{{ x.foo is undefined }}", x => HashMap::<String, String>::new()),
        "True"
    );

    // Everything else behaves exactly as `Strict` does, asserted against
    // `Strict` itself rather than against a hardcoded error kind -- the kinds
    // are not all `UndefinedError` (`{{ undefined|list }}` is an
    // `InvalidOperation`), and pinning them here would be pinning a detail of
    // a mode this variant only promises to agree with.
    let mut strict = Environment::new();
    strict.set_undefined_behavior(UndefinedBehavior::Strict);

    for expression in [
        "{{ undefined }}",
        "{{ undefined.missing_attribute }}",
        "{% for x in undefined %}...{% endfor %}",
        "{{ not undefined }}",
        "{% if undefined %}y{% endif %}",
        "{{ undefined|list }}",
        "{{ undefined|upper }}",
        "{{ 'x' ~ undefined }}",
    ] {
        let chainable = env.render_str(expression, ()).map_err(|e| e.kind());
        let strictly = strict.render_str(expression, ()).map_err(|e| e.kind());

        assert!(chainable.is_err(), "{expression} should have failed");
        assert_eq!(chainable, strictly, "{expression} should match Strict");
    }

    // And `default` still catches the bare one, so a template can defend
    // itself at either depth.
    assert_eq!(render!(in env, "{{ undefined|default('D') }}"), "D");

    // A custom formatter does not open a hole: the engine refuses before it
    // is consulted.
    env.set_formatter(|out, _state, value| {
        use std::fmt::Write;
        write!(out, "[{value}]").map_err(minijinja::Error::from)
    });
    assert_eq!(
        env.render_str("{{ undefined }}", ()).unwrap_err().kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(render!(in env, "{{ 1 }}"), "[1]");
}

#[test]
fn test_semi_strict_undefined() {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::SemiStrict);
    env.add_filter(
        "test_rest_join",
        |_state: &State, values: minijinja::value::Rest<String>| -> String { values.join("|") },
    );
    env.add_filter(
        "test_vec_join",
        |_state: &State, values: Vec<String>| -> String { values.join("|") },
    );

    assert_eq!(
        env.render_str("{{ true.missing_attribute }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined.missing_attribute }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("<{% for x in undefined %}...{% endfor %}>", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ 'foo' is in(undefined) }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(render!(in env, "<{% if undefined %}42{% endif %}>"), "<>");
    assert_eq!(
        env.render_str("<{{ undefined }}>", ()).unwrap_err().kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(render!(in env, "{{ not undefined }}"), "True");
    assert_eq!(render!(in env, "{{ undefined is undefined }}"), "True");
    assert_eq!(render!(in env, "<{{ 42 if false }}>"), "<>");
    assert_eq!(
        render!(in env, "{{ x.foo is undefined }}", x => HashMap::<String, String>::new()),
        "True"
    );
    assert_eq!(
        env.render_str(
            "<{% if x.foo %}...{% endif %}>",
            context! { x => HashMap::<String, String>::new() }
        )
        .unwrap(),
        "<>"
    );
    assert_eq!(
        env.render_str("{{ undefined|list }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidOperation
    );
    assert_eq!(
        env.render_str("{{ 42 in undefined }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|upper }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|int }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|float }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|string }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|test_rest_join }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ [undefined]|test_vec_join }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    // bool follows is_true semantics: SemiStrict allows it (returns false), Strict errors
    assert_eq!(render!(in env, "{{ undefined|bool }}"), "False");
    // none|int should still work (undefined check only applies to undefined, not none)
    assert_eq!(render!(in env, "{{ none|int }}"), "0");
    assert_eq!(
        env.render_str("{{ undefined|default('FALLBACK') }}", ())
            .unwrap(),
        "FALLBACK"
    );
    assert_eq!(
        env.render_str("{{ foo == 'foo' }}", ()).unwrap_err().kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ foo ~ 'x' }}", ()).unwrap_err().kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ foo|default(bar) }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
}

#[test]
fn test_strict_undefined() {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_filter(
        "test_rest_join",
        |_state: &State, values: minijinja::value::Rest<String>| -> String { values.join("|") },
    );
    env.add_filter(
        "test_vec_join",
        |_state: &State, values: Vec<String>| -> String { values.join("|") },
    );

    assert_eq!(
        env.render_str("{{ true.missing_attribute }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined.missing_attribute }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("<{% for x in undefined %}...{% endfor %}>", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ 'foo' is in(undefined) }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("<{% if undefined %}42{% endif %}>", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("<{{ undefined }}>", ()).unwrap_err().kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("<{{ not undefined }}>", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(render!(in env, "{{ undefined is undefined }}"), "True");
    assert_eq!(env.render_str("<{{ 42 if false }}>", ()).unwrap(), "<>");
    assert_eq!(
        render!(in env, "{{ x.foo is undefined }}", x => HashMap::<String, String>::new()),
        "True"
    );
    assert_eq!(
        env.render_str(
            "{% if x.foo %}...{% endif %}",
            context! { x => HashMap::<String, String>::new() }
        )
        .unwrap_err()
        .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|list }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidOperation
    );
    assert_eq!(
        env.render_str("{{ 42 in undefined }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined in [1, 2, 3] }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined in 'abc' }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|upper }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|int }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|float }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|bool }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|string }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ undefined|test_rest_join }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    assert_eq!(
        env.render_str("{{ [undefined]|test_vec_join }}", ())
            .unwrap_err()
            .kind(),
        ErrorKind::UndefinedError
    );
    // none|int should still work (only undefined is rejected)
    assert_eq!(render!(in env, "{{ none|int }}"), "0");
    assert_eq!(
        env.render_str("{{ undefined|default('FALLBACK') }}", ())
            .unwrap(),
        "FALLBACK"
    );
}

#[test]
fn test_chainable_undefined() {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Chainable);
    env.add_filter("test", |state: &State, value: String| -> String {
        assert_eq!(state.undefined_behavior(), UndefinedBehavior::Chainable);
        assert_eq!(value, "");
        value
    });

    assert_eq!(render!(in env, "<{{ true.missing_attribute }}>"), "<>");
    assert_eq!(render!(in env, "<{{ undefined.missing_attribute }}>"), "<>");
    assert_eq!(
        render!(in env, "<{% for x in undefined %}...{% endfor %}>"),
        "<>"
    );
    assert_eq!(
        render!(in env, "{{ x.foo is undefined }}", x => HashMap::<String, String>::new()),
        "True"
    );
    assert_eq!(render!(in env, "{{ 'foo' is in(undefined) }}"), "False");
    assert_eq!(render!(in env, "<{{ undefined }}>"), "<>");
    assert_eq!(render!(in env, "{{ not undefined }}"), "True");
    assert_eq!(render!(in env, "{{ undefined is undefined }}"), "True");
    assert_eq!(render!(in env, "{{ undefined|list }}"), "[]");
    assert_eq!(render!(in env, "<{{ undefined|test }}>"), "<>");
    assert_eq!(render!(in env, "{{ 42 in undefined }}"), "False");
}
