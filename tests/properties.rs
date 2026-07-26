use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn vm_and_tree_agree_for_generated_integer_expressions(
        left in -1_000_000i64..1_000_000,
        middle in -1_000_000i64..1_000_000,
        right in -1_000_000i64..1_000_000,
    ) {
        let source = format!("({left} + {middle}) - {right}");
        let tokens = nivren::lexer::scan(&source).unwrap();
        let program = nivren::parser::parse(tokens).unwrap();
        nivren::typecheck::check(&program).unwrap();
        let expected = nivren::runtime::Interpreter::new().run(&program).unwrap();
        let chunk = nivren::bytecode::compile(&program).unwrap();
        let actual = nivren::runtime::Interpreter::new().run_bytecode(&chunk).unwrap();
        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn formatter_is_idempotent_for_generated_bindings(
        name in "[a-z][a-z0-9_]{0,15}",
        value in any::<i64>(),
    ) {
        let source = format!("keep   {name}:Int={value};\n{name}");
        let once = nivren::formatter::format(&source);
        let twice = nivren::formatter::format(&once);
        prop_assert_eq!(once, twice);
    }

    #[test]
    fn arbitrary_source_never_panics(source in ".{0,4096}") {
        let result = std::panic::catch_unwind(|| {
            if let Ok(tokens) = nivren::lexer::scan(&source) {
                if let Ok(program) = nivren::parser::parse(tokens) {
                    let _ = nivren::typecheck::check(&program);
                }
            }
        });
        prop_assert!(result.is_ok());
    }
}
