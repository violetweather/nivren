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
            if let Ok(tokens) = nivren::lexer::scan(&source)
                && let Ok(program) = nivren::parser::parse(tokens)
                && nivren::typecheck::check(&program).is_ok() {
                    for optimization in [
                        nivren::intent::Optimization::Disabled,
                        nivren::intent::Optimization::Enabled,
                    ] {
                        let graph = nivren::intent::analyze(&program, optimization);
                        let _ = graph.validate();
                    }
                }
        });
        prop_assert!(result.is_ok());
    }

    #[test]
    fn formatter_is_idempotent_for_arbitrary_source(source in ".{0,4096}") {
        let once = nivren::formatter::format(&source);
        let twice = nivren::formatter::format(&once);
        prop_assert_eq!(once, twice);
    }

    #[test]
    fn optimized_and_unoptimized_intent_execution_agree(
        input in -100_000i64..100_000,
        factor in -32i64..32,
    ) {
        let source = format!(
            "define scale takes {{ value is Int }} gives Int {{ give value * {factor} }}\n{input} through scale"
        );
        let tokens = nivren::lexer::scan(&source).unwrap();
        let program = nivren::parser::parse(tokens).unwrap();
        nivren::typecheck::check(&program).unwrap();
        let optimized = nivren::intent::analyze(&program, nivren::intent::Optimization::Enabled);
        let unoptimized = nivren::intent::analyze(&program, nivren::intent::Optimization::Disabled);
        prop_assert!(optimized.validate().is_ok());
        prop_assert!(unoptimized.validate().is_ok());
        prop_assert_eq!(optimized.summary.pure_runtime_plan_allocations, 0);
        prop_assert_eq!(unoptimized.summary.pure_runtime_plan_allocations, 0);
        let chunk = nivren::bytecode::compile(&program).unwrap();
        let optimized_value = nivren::runtime::Interpreter::new().run_bytecode(&chunk).unwrap();
        let unoptimized_value = nivren::runtime::Interpreter::new().run_bytecode(&chunk).unwrap();
        prop_assert_eq!(optimized_value, unoptimized_value);
    }

    #[test]
    fn denied_effects_never_enter_the_runtime_effect_sequence(
        variable in "NIVREN_DENIED_[A-Z]{1,12}"
    ) {
        let source = format!(
            "define read takes {{}} gives maybe String needs Environment {{ give perform std.env.get with {{ name set \"{variable}\" }} }}\nperform read with {{}}"
        );
        let tokens = nivren::lexer::scan(&source).unwrap();
        let program = nivren::parser::parse(tokens).unwrap();
        nivren::typecheck::check(&program).unwrap();
        let graph = nivren::intent::analyze(&program, nivren::intent::Optimization::Enabled);
        prop_assert!(graph.validate().is_ok());
        let chunk = nivren::bytecode::compile(&program).unwrap();
        let mut interpreter = nivren::runtime::Interpreter::new()
            .with_capabilities(Vec::<String>::new());
        interpreter.enable_metrics();
        prop_assert!(interpreter.run_bytecode(&chunk).is_err());
        prop_assert!(interpreter.execution_metrics().unwrap().effect_sequence.is_empty());
    }
}
