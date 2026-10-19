//! `Object::contains_key`: a containment check on a map object that computes
//! its values when they are looked up.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use minijinja::value::{Enumerator, Object, ObjectRepr, Value};
use minijinja::{context, Environment};

/// A map whose one value is computed when it is looked up, counting how
/// often that happens.
#[derive(Debug, Default)]
struct Computed {
    lookups: AtomicUsize,
    answers_from_keys: bool,
}

impl Object for Computed {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Map
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        if key.as_str() != Some("token") {
            return None;
        }
        self.lookups.fetch_add(1, Ordering::SeqCst);
        Some(Value::from("computed"))
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Str(&["token"])
    }

    fn contains_key(self: &Arc<Self>, key: &Value) -> bool {
        if self.answers_from_keys {
            key.as_str() == Some("token")
        } else {
            self.get_value(key).is_some()
        }
    }
}

fn check(answers_from_keys: bool, source: &str) -> (String, usize) {
    let map = Arc::new(Computed {
        answers_from_keys,
        ..Computed::default()
    });
    let env = Environment::new();
    let rendered = env
        .render_str(
            source,
            context! { m => Value::from_dyn_object(map.clone()) },
        )
        .unwrap();
    (rendered, map.lookups.load(Ordering::SeqCst))
}

#[test]
fn an_object_answering_from_its_keys_is_not_looked_up() {
    let source = "{{ 'token' in m }} {{ 'other' in m }} {{ 'token' not in m }}";
    assert_eq!(check(true, source), ("True False False".into(), 0));
}

#[test]
fn the_default_answers_by_looking_the_key_up() {
    let (rendered, lookups) = check(false, "{{ 'token' in m }} {{ 'other' in m }}");
    assert_eq!(rendered, "True False");
    assert_eq!(lookups, 1);
}

#[test]
fn reading_the_value_still_looks_it_up() {
    assert_eq!(check(true, "{{ m.token }}"), ("computed".into(), 1));
}
