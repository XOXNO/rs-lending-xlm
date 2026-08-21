#[test]
fn test_contractevent_macro_expansion() {
    macrotest::expand("tests/expand/*.rs");
}

#[test]
fn test_contractevent_compiles() {
    let t = trybuild::TestCases::new();
    t.pass("tests/expand/basic.rs");
    t.pass("tests/expand/generics.rs");
}
