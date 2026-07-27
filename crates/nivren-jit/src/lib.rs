use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, UserFuncName, types};
use cranelift_codegen::settings;
use cranelift_codegen::settings::Configurable;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntOp {
    Constant(i64),
    Load(u32),
    Define(u32),
    Store(u32),
    Pop,
    Add,
    Subtract,
    Multiply,
    Negate,
    Return,
}

pub struct CompiledFunction {
    _module: JITModule,
    function: unsafe extern "C" fn(*const i64, *mut u8) -> i64,
    parameters: usize,
}

pub struct AotObject;

impl AotObject {
    pub fn compile(
        name: &str,
        parameters: usize,
        slots: usize,
        operations: &[IntOp],
    ) -> Result<Vec<u8>, String> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(
                "AOT export name must contain only ASCII letters, digits, or underscore".into(),
            );
        }
        if parameters > slots || slots > 4096 || operations.len() > 1_000_000 {
            return Err("invalid AOT function limits".into());
        }
        let mut flags = settings::builder();
        flags
            .set("use_colocated_libcalls", "false")
            .map_err(|error| error.to_string())?;
        flags
            .set("is_pic", "true")
            .map_err(|error| error.to_string())?;
        let isa = cranelift_native::builder()
            .map_err(|error| error.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|error| error.to_string())?;
        let builder = ObjectBuilder::new(isa, "nivren_aot", default_libcall_names())
            .map_err(|error| error.to_string())?;
        let mut module = ObjectModule::new(builder);
        define_integer_function(
            &mut module,
            name,
            Linkage::Export,
            parameters,
            slots,
            operations,
        )?;
        module.finish().emit().map_err(|error| error.to_string())
    }
}

// SAFETY: The finalized module is never mutated after construction, its code and data
// allocations remain owned by this value, and Nivren stores each compiled function behind
// a Mutex. Moving that exclusive owner to another thread does not invalidate executable
// addresses or create concurrent access to JITModule's non-Sync internals.
unsafe impl Send for CompiledFunction {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError {
    Arity,
    Overflow,
}

impl CompiledFunction {
    pub fn compile(parameters: usize, slots: usize, operations: &[IntOp]) -> Result<Self, String> {
        if parameters > slots || slots > 4096 || operations.len() > 1_000_000 {
            return Err("invalid JIT function limits".into());
        }
        let mut flags = settings::builder();
        flags
            .set("use_colocated_libcalls", "false")
            .map_err(|error| error.to_string())?;
        flags
            .set("is_pic", "false")
            .map_err(|error| error.to_string())?;
        let isa = cranelift_native::builder()
            .map_err(|error| error.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|error| error.to_string())?;
        let builder = JITBuilder::with_isa(isa, default_libcall_names());
        let mut module = JITModule::new(builder);
        let pointer = module.target_config().pointer_type();
        let mut signature = module.make_signature();
        signature.params.push(AbiParam::new(pointer));
        signature.params.push(AbiParam::new(pointer));
        signature.returns.push(AbiParam::new(types::I64));
        let function_id = module
            .declare_function("nivren_hot", Linkage::Local, &signature)
            .map_err(|error| error.to_string())?;
        let mut context = module.make_context();
        context.func.signature = signature;
        context.func.name = UserFuncName::user(0, 0);
        let mut builder_context = FunctionBuilderContext::new();
        {
            let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let entry = function.create_block();
            let overflow = function.create_block();
            function.append_block_params_for_function_params(entry);
            function.switch_to_block(entry);
            function.seal_block(entry);
            let argument_pointer = function.block_params(entry)[0];
            let overflow_pointer = function.block_params(entry)[1];
            for slot in 0..slots {
                let variable = Variable::from_u32(u32::try_from(slot).unwrap());
                function.declare_var(variable, types::I64);
                let initial = if slot < parameters {
                    function.ins().load(
                        types::I64,
                        MemFlags::trusted(),
                        argument_pointer,
                        i32::try_from(slot * 8).map_err(|_| "JIT slot offset overflow")?,
                    )
                } else {
                    function.ins().iconst(types::I64, 0)
                };
                function.def_var(variable, initial);
            }
            let mut stack = Vec::new();
            let mut returned = false;
            for operation in operations {
                match operation {
                    IntOp::Constant(value) => {
                        stack.push(function.ins().iconst(types::I64, *value));
                    }
                    IntOp::Load(slot) => stack.push(function.use_var(Variable::from_u32(*slot))),
                    IntOp::Define(slot) | IntOp::Store(slot) => {
                        let value = *stack.last().ok_or("JIT stack underflow")?;
                        function.def_var(Variable::from_u32(*slot), value);
                    }
                    IntOp::Pop => {
                        stack.pop().ok_or("JIT stack underflow")?;
                    }
                    IntOp::Add | IntOp::Subtract | IntOp::Multiply => {
                        let right = stack.pop().ok_or("JIT stack underflow")?;
                        let left = stack.pop().ok_or("JIT stack underflow")?;
                        let result = match operation {
                            IntOp::Add => function.ins().iadd(left, right),
                            IntOp::Subtract => function.ins().isub(left, right),
                            IntOp::Multiply => function.ins().imul(left, right),
                            _ => unreachable!(),
                        };
                        let overflowed = match operation {
                            IntOp::Add => {
                                let first = function.ins().bxor(left, result);
                                let second = function.ins().bxor(right, result);
                                let both = function.ins().band(first, second);
                                function.ins().icmp_imm(
                                    cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                                    both,
                                    0,
                                )
                            }
                            IntOp::Subtract => {
                                let first = function.ins().bxor(left, right);
                                let second = function.ins().bxor(left, result);
                                let both = function.ins().band(first, second);
                                function.ins().icmp_imm(
                                    cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                                    both,
                                    0,
                                )
                            }
                            IntOp::Multiply => {
                                let high = function.ins().smulhi(left, right);
                                let sign = function.ins().sshr_imm(result, 63);
                                function.ins().icmp(
                                    cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                                    high,
                                    sign,
                                )
                            }
                            _ => unreachable!(),
                        };
                        branch_if_overflow(&mut function, overflowed, overflow);
                        stack.push(result);
                    }
                    IntOp::Negate => {
                        let value = stack.pop().ok_or("JIT stack underflow")?;
                        let overflowed = function.ins().icmp_imm(
                            cranelift_codegen::ir::condcodes::IntCC::Equal,
                            value,
                            i64::MIN,
                        );
                        branch_if_overflow(&mut function, overflowed, overflow);
                        stack.push(function.ins().ineg(value));
                    }
                    IntOp::Return => {
                        let value = stack.pop().ok_or("JIT stack underflow")?;
                        function.ins().return_(&[value]);
                        returned = true;
                        break;
                    }
                }
            }
            if !returned {
                return Err("JIT function has no give".into());
            }
            function.switch_to_block(overflow);
            function.seal_block(overflow);
            let one = function.ins().iconst(types::I8, 1);
            function
                .ins()
                .store(MemFlags::trusted(), one, overflow_pointer, 0);
            let zero = function.ins().iconst(types::I64, 0);
            function.ins().return_(&[zero]);
            function.finalize();
        }
        module
            .define_function(function_id, &mut context)
            .map_err(|error| error.to_string())?;
        module.clear_context(&mut context);
        module
            .finalize_definitions()
            .map_err(|error| error.to_string())?;
        let pointer = module.get_finalized_function(function_id);
        let function = unsafe {
            std::mem::transmute::<*const u8, unsafe extern "C" fn(*const i64, *mut u8) -> i64>(
                pointer,
            )
        };
        Ok(Self {
            _module: module,
            function,
            parameters,
        })
    }

    pub fn call(&self, arguments: &[i64]) -> Result<i64, CallError> {
        if arguments.len() != self.parameters {
            return Err(CallError::Arity);
        }
        let mut overflow = 0u8;
        let value = unsafe { (self.function)(arguments.as_ptr(), &mut overflow) };
        if overflow == 0 {
            Ok(value)
        } else {
            Err(CallError::Overflow)
        }
    }
}

fn branch_if_overflow(
    function: &mut FunctionBuilder<'_>,
    condition: cranelift_codegen::ir::Value,
    overflow: cranelift_codegen::ir::Block,
) {
    let next = function.create_block();
    function.ins().brif(condition, overflow, &[], next, &[]);
    function.switch_to_block(next);
    function.seal_block(next);
}

fn define_integer_function<M: Module>(
    module: &mut M,
    name: &str,
    linkage: Linkage,
    parameters: usize,
    slots: usize,
    operations: &[IntOp],
) -> Result<(), String> {
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.params.push(AbiParam::new(pointer));
    signature.returns.push(AbiParam::new(types::I64));
    let function_id = module
        .declare_function(name, linkage, &signature)
        .map_err(|error| error.to_string())?;
    let mut context = module.make_context();
    context.func.signature = signature;
    context.func.name = UserFuncName::user(0, 0);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut function = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let entry = function.create_block();
        let overflow = function.create_block();
        function.append_block_params_for_function_params(entry);
        function.switch_to_block(entry);
        function.seal_block(entry);
        let argument_pointer = function.block_params(entry)[0];
        let overflow_pointer = function.block_params(entry)[1];
        for slot in 0..slots {
            let variable = Variable::from_u32(u32::try_from(slot).unwrap());
            function.declare_var(variable, types::I64);
            let initial = if slot < parameters {
                function.ins().load(
                    types::I64,
                    MemFlags::trusted(),
                    argument_pointer,
                    i32::try_from(slot * 8).map_err(|_| "AOT slot offset overflow")?,
                )
            } else {
                function.ins().iconst(types::I64, 0)
            };
            function.def_var(variable, initial);
        }
        let mut stack = Vec::new();
        let mut returned = false;
        for operation in operations {
            match operation {
                IntOp::Constant(value) => stack.push(function.ins().iconst(types::I64, *value)),
                IntOp::Load(slot) => stack.push(function.use_var(Variable::from_u32(*slot))),
                IntOp::Define(slot) | IntOp::Store(slot) => {
                    let value = *stack.last().ok_or("AOT stack underflow")?;
                    function.def_var(Variable::from_u32(*slot), value);
                }
                IntOp::Pop => {
                    stack.pop().ok_or("AOT stack underflow")?;
                }
                IntOp::Add | IntOp::Subtract | IntOp::Multiply => {
                    let right = stack.pop().ok_or("AOT stack underflow")?;
                    let left = stack.pop().ok_or("AOT stack underflow")?;
                    let result = match operation {
                        IntOp::Add => function.ins().iadd(left, right),
                        IntOp::Subtract => function.ins().isub(left, right),
                        IntOp::Multiply => function.ins().imul(left, right),
                        _ => unreachable!(),
                    };
                    let overflowed = match operation {
                        IntOp::Add => {
                            let first = function.ins().bxor(left, result);
                            let second = function.ins().bxor(right, result);
                            let both = function.ins().band(first, second);
                            function.ins().icmp_imm(
                                cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                                both,
                                0,
                            )
                        }
                        IntOp::Subtract => {
                            let first = function.ins().bxor(left, right);
                            let second = function.ins().bxor(left, result);
                            let both = function.ins().band(first, second);
                            function.ins().icmp_imm(
                                cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                                both,
                                0,
                            )
                        }
                        IntOp::Multiply => {
                            let high = function.ins().smulhi(left, right);
                            let sign = function.ins().sshr_imm(result, 63);
                            function.ins().icmp(
                                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                                high,
                                sign,
                            )
                        }
                        _ => unreachable!(),
                    };
                    branch_if_overflow(&mut function, overflowed, overflow);
                    stack.push(result);
                }
                IntOp::Negate => {
                    let value = stack.pop().ok_or("AOT stack underflow")?;
                    let overflowed = function.ins().icmp_imm(
                        cranelift_codegen::ir::condcodes::IntCC::Equal,
                        value,
                        i64::MIN,
                    );
                    branch_if_overflow(&mut function, overflowed, overflow);
                    stack.push(function.ins().ineg(value));
                }
                IntOp::Return => {
                    let value = stack.pop().ok_or("AOT stack underflow")?;
                    function.ins().return_(&[value]);
                    returned = true;
                    break;
                }
            }
        }
        if !returned {
            return Err("AOT function has no give".into());
        }
        function.switch_to_block(overflow);
        function.seal_block(overflow);
        let one = function.ins().iconst(types::I8, 1);
        function
            .ins()
            .store(MemFlags::trusted(), one, overflow_pointer, 0);
        let zero = function.ins().iconst(types::I64, 0);
        function.ins().return_(&[zero]);
        function.finalize();
    }
    module
        .define_function(function_id, &mut context)
        .map_err(|error| error.to_string())?;
    module.clear_context(&mut context);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AotObject, CallError, CompiledFunction, IntOp};

    #[test]
    fn native_integer_code_executes_and_preserves_overflow() {
        let function = CompiledFunction::compile(
            2,
            2,
            &[IntOp::Load(0), IntOp::Load(1), IntOp::Add, IntOp::Return],
        )
        .unwrap();
        assert_eq!(function.call(&[20, 22]), Ok(42));
        assert_eq!(function.call(&[i64::MAX, 1]), Err(CallError::Overflow));
    }

    #[test]
    fn native_aot_objects_are_deterministic_and_nonempty() {
        let operations = [
            IntOp::Load(0),
            IntOp::Constant(2),
            IntOp::Multiply,
            IntOp::Return,
        ];
        let first = AotObject::compile("nivren_double", 1, 1, &operations).unwrap();
        let second = AotObject::compile("nivren_double", 1, 1, &operations).unwrap();
        assert!(first.len() > 64);
        assert_eq!(first, second);
        assert!(AotObject::compile("invalid-name", 1, 1, &operations).is_err());
    }
}
