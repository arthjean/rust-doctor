#![allow(dead_code)]

fn helper() {}

#[test]
fn probe() {
    suppression_attribute_scope::uses_the_modules();
}
